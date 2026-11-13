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
    if ENGINE.get().is_none() {
        set_for_tests();
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

/// A crate's own test binary, which the engine does not load: identities of that binary (its path and when it was
/// built), so that nothing a store kept under another build is taken as this one's.
#[cfg(any(test, feature = "test-hooks"))]
fn set_for_tests() {
    use std::hash::{Hash, Hasher};
    let exe = std::env::current_exe().unwrap_or_default();
    let built = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
    let id = |salt: &str| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (salt, &exe, built).hash(&mut h);
        h.finish()
    };
    let hex = |salt: &str| format!("{:016x}{:016x}", id(salt), id(&format!("{salt}+")));
    set(Box::leak(hex("engine").into_boxed_str()), Box::leak(hex("grammar").into_boxed_str()));
}

/// The grammars' build identity as a number: its first 15 hex digits.
pub fn grammar_version() -> i64 {
    i64::from_str_radix(&grammar()[..15], 16).expect("grammar build identity is hex")
}
