//! The engine's build identities (`build.rs`: `RERAIL_ENGINE_BUILD_ID` covers every engine source file and the
//! locked dependencies, `RERAIL_GRAMMAR_BUILD_ID` the parser and grammar versions a stored tape was parsed with).
//! The engine's build script makes them; the engine sets them here as it loads (`crate::install_build_ids`), so
//! every crate reads the engine's, not its own.
use std::sync::OnceLock;

static ENGINE: OnceLock<&'static str> = OnceLock::new();
static GRAMMAR: OnceLock<&'static str> = OnceLock::new();

/// Sets both identities; the first setting holds.
pub fn set(engine: &'static str, grammar: &'static str) {
    let _ = ENGINE.set(engine);
    let _ = GRAMMAR.set(grammar);
}

/// The engine's build identity.
pub fn engine() -> &'static str {
    #[cfg(any(test, feature = "test-hooks"))]
    {
        if let Some(rebuilt) = crate::code_rule_dsl::logic::profile::bound::<Rebuilt>().0 {
            return rebuilt;
        }
        if ENGINE.get().is_none() {
            set_for_tests();
        }
    }
    ENGINE.get().copied().expect("the engine sets its build identity as it loads")
}

/// The parser and grammar build identity.
pub fn grammar() -> &'static str {
    #[cfg(any(test, feature = "test-hooks"))]
    if GRAMMAR.get().is_none() {
        set_for_tests();
    }
    GRAMMAR.get().copied().expect("the engine sets its build identity as it loads")
}

/// The engine's identity a test's runs take as another build's (`rebuild_for_tests`), in the run's context.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone, Copy, Default)]
struct Rebuilt(Option<&'static str>);

/// A test's next runs as another build of the engine's (the next of the one they ran as), over the stores the earlier
/// runs left: what a rebuild of the engine gives a store's next run, until the guard drops. The grammar's identity
/// stays. Bound in the test's context, so its runs and their jobs take it and no other test's do: set for the process,
/// it made the runs of the tests beside it in one process (`cargo test`'s threads) find their own entries an older
/// build's (`partial_tests`, four red at 20 threads).
#[cfg(any(test, feature = "test-hooks"))]
#[must_use = "the runs are another build's only while the guard is held"]
pub fn rebuild_for_tests() -> crate::code_rule_dsl::logic::profile::Bound {
    let now = engine();
    let next: &'static str = Box::leak(format!("{now}+rebuilt").into_boxed_str());
    crate::code_rule_dsl::logic::profile::bind(Rebuilt(Some(next)))
}

/// A crate's own test binary, which the engine does not load: identities of that binary (its path and when it was
/// built), so that nothing a store kept under another build is taken as this one's.
#[cfg(any(test, feature = "test-hooks"))]
fn set_for_tests() {
    use std::hash::{Hash, Hasher};
    let exe = std::env::current_exe().unwrap_or_default();
    let built = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
    let sources = sources_stamp();
    let id = |salt: &str| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (salt, &exe, built, sources).hash(&mut h);
        h.finish()
    };
    let hex = |salt: &str| format!("{:016x}{:016x}", id(salt), id(&format!("{salt}+")));
    set(Box::leak(hex("engine").into_boxed_str()), Box::leak(hex("grammar").into_boxed_str()));
}

/// The engine's sources as a test build reads them: each file's path, size and modification time under what
/// `build.rs`'s identity covers (the engine's sources, the crates it builds on, the language registry, the rule
/// library and the lock), by `stat` alone. The binary's own identity is not enough: under Relax every checkout's
/// tests run in one host binary that an edit does not rebuild, so stores and templates kept by other code (another
/// checkout's, or this one's before an edit) were taken as this build's.
#[cfg(any(test, feature = "test-hooks"))]
fn sources_stamp() -> u64 {
    use std::hash::{Hash, Hasher};
    fn walk(dir: &std::path::Path, root: &std::path::Path, h: &mut std::collections::hash_map::DefaultHasher) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name();
            if matches!(name.to_str(), Some("target" | "vendor" | "tmp" | "node_modules" | ".git")) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let path = e.path();
            if meta.is_dir() {
                walk(&path, root, h);
            } else {
                (path.strip_prefix(root).unwrap_or(&path), meta.len(), meta.modified().ok()).hash(h);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for part in ["native/src", "native/crates", "native/build.rs", "native/Cargo.toml", "accel", "gpu", "languages", "docs/rule-syntax", "Cargo.lock"] {
        let path = root.join(part);
        if path.is_dir() {
            walk(&path, &root, &mut h);
        } else if let Ok(meta) = std::fs::metadata(&path) {
            (part, meta.len(), meta.modified().ok()).hash(&mut h);
        }
    }
    h.finish()
}

/// The grammars' build identity as a number: its first 15 hex digits.
pub fn grammar_version() -> i64 {
    i64::from_str_radix(&grammar()[..15], 16).expect("grammar build identity is hex")
}
