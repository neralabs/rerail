// Inside `src/` so every copy of the native sources (release builds) carries it; the
// library compiles it only for tests.
#[path = "src/build_identity.rs"]
mod identity;
#[path = "src/build_languages.rs"]
mod languages;
#[path = "src/build_grammars.rs"]
mod grammars;

use grammars::{GRAMMARS, LFS_POINTER, LFS_REMEDY, collect_sources};
use identity::{grammar_build_id, hex};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    check_compiler_cache();
    // The browser build (wasm32-wasip1) is no Node addon: napi-build would want emnapi.
    if !std::env::var("TARGET").unwrap_or_default().starts_with("wasm32") {
        napi_build::setup();
    }
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    check_names_table(&manifest);
    let omitted = omitted_languages();
    // The crate that declares the grammars' symbols builds them: this one, until the crate split gives them to
    // `rerail-lang` (tools/cratify sets this to false and writes that crate's build script).
    const BUILDS_GRAMMARS: bool = false;
    if BUILDS_GRAMMARS {
        grammars::build_all(&manifest, &omitted);
    }
    let registry = generate_languages(&omitted);
    configure_fixtures();
    build_identities(&registry);
}

/// On a machine with the build farm (`scripts/build/farm`: its configuration exists), every build
/// goes through the farm's wrapper, the one compiler cache every worktree shares: another wrapper,
/// or none, would compile again what the other worktrees compiled. The farm's `setup` makes it
/// the machine's (`~/.cargo/config.toml`); this stops a build that overrode it (`RUSTC_WRAPPER`
/// exported). Relax (`RELAX_STD` set) builds with its own (itself, or none), and `RERAIL_FARM_ANY_ENV=1` allows any
/// (a measurement of the compiler itself). Elsewhere (CI, a machine without the farm) it does nothing.
fn check_compiler_cache() {
    for name in ["RUSTC_WRAPPER", "RERAIL_FARM_ANY_ENV", "RELAX_STD"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let config = std::env::var_os("RERAIL_FARM_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config/rerail-farm/farm.json"));
    if !config.exists() || std::env::var("RERAIL_FARM_ANY_ENV").as_deref() == Ok("1") {
        return;
    }
    let farm = home.join(".local/share/rerail-farm/rustc-wrapper");
    let wrapper = std::env::var_os("RUSTC_WRAPPER").unwrap_or_default();
    // Relax drives its own cargo pass with itself as the wrapper (`RUSTC_WRAPPER=.../bin/relax`); its
    // builds are outside the farm by design (AGENTS.md, "Relax: the live test loop").
    let relax = std::env::var_os("RELAX_STD").is_some()
        && (wrapper.is_empty() || Path::new(&wrapper).file_name().is_some_and(|n| n == "relax"));
    if PathBuf::from(&wrapper) == farm || relax {
        return;
    }
    panic!(
        "this machine has the build farm, but this build's RUSTC_WRAPPER is {wrapper:?}, not the farm's \
         ({}): it would not reuse what the other worktrees compiled. Do not set RUSTC_WRAPPER (the farm's \
         setup puts the farm's in ~/.cargo/config.toml); RERAIL_FARM_ANY_ENV=1 allows a build that must \
         differ (scripts/build/farm/README.md, \"One configuration\").",
        farm.display()
    );
}


/// The registry's supported languages this build leaves out: each language `<id>` is built only with its feature
/// `lang-<id>` (all are by default; the browser build picks the ones a page needs, `wasm/README.md`).
fn omitted_languages() -> Vec<String> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    languages::omitted_languages(&manifest.parent().expect("repository root").join("languages"))
}


/// The language registry, `languages/` at the repository root: validated, and generated into
/// `OUT_DIR/language_registry.rs`. Returns each registry file and its bytes. The languages only
/// tests see (`native/test-languages/`, absent from a copy that carries none) are validated
/// against it and generated as `#[cfg(test)]` items; they are not returned, so they are no part of
/// the engine's identity.
/// The `omitted` languages keep their tables but no grammar (`language_registry.rs`'s `built`).
fn generate_languages(omitted: &[String]) -> Vec<(String, Vec<u8>)> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let vendored = GRAMMARS.iter().map(|grammar| grammar.name).collect::<Vec<_>>();
    assert_eq!(vendored, languages::VENDORED, "GRAMMARS and build_languages.rs's VENDORED name the same grammars");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate_registry(&manifest, omitted, &out)
}


/// The Unicode names table `include_bytes!` embeds (`value/names.rs`): it must be the table, not
/// its Git LFS pointer, or every Python `\N{...}` escape fails at run time.
fn check_names_table(manifest: &Path) {
    let relative = "src/code_rule_dsl/value/names/names.bin";
    let path = manifest.join(relative);
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("native/{relative} is unreadable ({error}): run `{LFS_REMEDY}`")
    });
    if bytes.starts_with(LFS_POINTER) {
        panic!(
            "native/{relative} is a Git LFS pointer, not the names table: run `{LFS_REMEDY}` (CONTRIBUTING.md)"
        );
    }
    if !bytes.starts_with(b"TNAMES01") {
        panic!(
            "native/{relative} is not a names table (no TNAMES01 header): run `{LFS_REMEDY}`, or regenerate it with scripts/generate/value/names.py"
        );
    }
}


/// Where the engine's tests read the conformance fixtures: `RERAIL_ORACLES_DIR` if set (relative
/// to the repository root), else the repository's `fixtures/` directory. Only tests read them, so a
/// build without them (a sparse checkout) goes on; the tests that need them then fail to compile,
/// naming the fix.
///
/// Without the override the script sets nothing: `.cargo/config.toml` gives every crate
/// `RERAIL_FIXTURES_DIR` (the same `fixtures/`, relative to the workspace). Setting it here too, as
/// the canonical path on disk, made the engine crate's `env!` input differ per checkout, so a Relax
/// run in one worktree made the engine's stores stale for every other worktree's runs, which share
/// them under `RELAX_SHARE` (scripts/dev/relax/README.md, invariant 5).
fn configure_fixtures() {
    println!("cargo:rerun-if-env-changed=RERAIL_ORACLES_DIR");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest.parent().expect("repository root");
    let Some(directory) = std::env::var_os("RERAIL_ORACLES_DIR") else {
        let fixtures = root.join("fixtures");
        println!("cargo:rerun-if-changed={}", fixtures.display());
        if !is_checkout(&fixtures) {
            println!(
                "cargo:warning=Rerail fixtures not found in {}: check out the repository's fixtures/ directory, or set RERAIL_ORACLES_DIR; the engine's tests need them",
                fixtures.display()
            );
        }
        return;
    };
    let directory = root.join(directory);
    assert!(
        is_checkout(&directory),
        "RERAIL_ORACLES_DIR={} is not a fixtures directory",
        directory.display()
    );
    let directory = directory
        .canonicalize()
        .expect("fixtures directory is readable");
    println!("cargo:rustc-env=RERAIL_ORACLES_DIR={}", directory.display());
    // What the tests' `fixtures_dir!` reads (`code_rule_dsl/fixtures.rs`); the engine's crates have it from
    // `.cargo/config.toml`, the engine from here, so an `RERAIL_ORACLES_DIR` override holds for its tests.
    println!("cargo:rustc-env=RERAIL_FIXTURES_DIR={}", directory.display());
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
    // A `.rr` tape is the tree the rule engine's parser builds, numbered by its kinds table (decision 0061).
    view_sources.push(manifest.join("crates/rr_syntax/src/syntax.rs"));
    view_sources.push(manifest.join("crates/rr_syntax/src/tape.rs"));
    // A named-only tape (RER-131) leaves out the tokens its grammar's automata recognize: the
    // automata, and the grammars they are built from, decide what a stored tape means.
    view_sources.push(manifest.join("src/code_rule_dsl/tape/tokens.rs"));
    view_sources.push(manifest.join("grammars/token-grammars/rust/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/php/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/java/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/csharp/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/html/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/css/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/go/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/c/grammar.json"));
    view_sources.push(manifest.join("grammars/token-grammars/cpp/grammar.json"));
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

