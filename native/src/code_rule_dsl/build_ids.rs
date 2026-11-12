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
    ENGINE.get().copied().expect("the engine sets its build identity as it loads")
}

/// The parser and grammar build identity.
pub fn grammar() -> &'static str {
    GRAMMAR.get().copied().expect("the engine sets its build identity as it loads")
}

/// The grammars' build identity as a number: its first 15 hex digits.
pub fn grammar_version() -> i64 {
    i64::from_str_radix(&grammar()[..15], 16).expect("grammar build identity is hex")
}
