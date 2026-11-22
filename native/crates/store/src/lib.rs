//! The engine's content store (`content_store`): sources, tapes, memos and run records in SQLite, the
//! `ContentStore` and every module that writes and reads through it. Over its base (rerail-store-base) and its
//! observer and git (rerail-observe), which it re-exports at their old paths (`content_store::*`, `git_access`,
//! `git_workspace`), so no caller changed.
//!
//! The sources are the engine's, in place (`native/src/content_store`): only the build moved. What the store needs
//! of the engine, the engine hands it as it loads: the SQL functions every connection gets
//! ([`content_store::set_connection_functions`]).

// The store's files name the base's trait as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate self as rerail_store;

pub use rerail_observe::{Canonical, git_access, git_workspace, probes, source_tree};
/// Git and worktree discovery's tests (rerail-observe's `git_workspace`): they publish to the store.
#[cfg(test)]
#[path = "../../../src/git_workspace_tests.rs"]
mod git_workspace_tests;

#[path = "../../../src/content_store/mod.rs"]
pub mod content_store;

/// The observer's tests (rerail-observe's `observe`): they read the store, and the kept facts it registers. Mounted at
/// the observer's depth, so their paths into the store (`super::super::journal`) read as they did.
#[cfg(test)]
#[path = "../../../src/content_store"]
mod observe_host {
    pub use crate::content_store::*;
    #[path = "observe"]
    pub mod observe {
        pub use crate::content_store::observe::*;
        // `observe.rs`'s own imports, which its tests read (`use super::*`).
        #[allow(unused_imports)]
        use crate::git_workspace::snapshot as source;
        #[allow(unused_imports)]
        use crate::content_store::Cached as _;
        #[allow(unused_imports)]
        use crate::content_store::RACY_NS;
        #[allow(unused_imports)]
        use rusqlite::{Connection, params};
        #[allow(unused_imports)]
        use sha2::{Digest as _, Sha256};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
        #[allow(unused_imports)]
        use std::path::{Path, PathBuf};
        #[allow(unused_imports)]
        use std::sync::{Arc, Mutex};
        #[allow(unused_imports)]
        use crate::content_store::observe::kept as facts;
        mod tests;
    }
}

/// This crate's own test binary, which boot does not load: what the store hands the observer below it is registered as
/// the binary loads, as boot registers it (`content_store::register_below`).
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::content_store::register_below();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
