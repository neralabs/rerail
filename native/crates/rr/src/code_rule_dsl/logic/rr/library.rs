//! Where the language's library is: the directory `#include` reads from.

use std::path::PathBuf;

/// The library `#include` reads from.
pub fn library() -> PathBuf {
    // `RERAIL_RR_LIB` names another copy: the browser build (native/wasm) mounts the library in its own filesystem.
    std::env::var_os("RERAIL_RR_LIB")
        .map(PathBuf::from)
        // From the engine's root (its checkout, or the installed package), with no `..` in the path: a path the engine
        // reads is named one way, so the file observer keeps one answer and one journal root for each directory
        // (`native/../docs` and `docs` were two).
        .unwrap_or_else(rerail_observe::source_tree::rule_library)
}
