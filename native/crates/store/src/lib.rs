//! The engine's content store and the git workspace it reads, as a crate of their own.
//!
//! The sources are the engine's, in place (`native/src/content_store`, `native/src/git_workspace.rs`): only
//! the build moved. The engine re-exports both modules at their old paths (`crate::content_store`,
//! `crate::git_workspace`), so no caller changed. What the store needs of the engine, the engine hands it as
//! it loads: the SQL functions every connection gets ([`content_store::set_connection_functions`]).

// The store's files name the trait below as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate self as rerail_store;

/// A path's canonical form without Windows' verbatim prefix (`dunce`: std's `\\?\C:\...` is no path `/`-joined
/// text or a comparison with a plain path finds); std's `canonicalize` elsewhere. The engine's and the store's every
/// canonical path (`.canonical()`, not `.canonicalize()`).
pub trait Canonical {
    fn canonical(&self) -> std::io::Result<std::path::PathBuf>;
}

impl Canonical for std::path::Path {
    fn canonical(&self) -> std::io::Result<std::path::PathBuf> {
        dunce::canonicalize(self)
    }
}

#[path = "../../../src/content_store/mod.rs"]
pub mod content_store;
/// Every in-process Git read: the one opener, its setup and its pool (`native/src/git_access`). Its Node-API
/// owner (`owner.rs`) is the engine's, which installs the setup.
#[path = "../../../src/git_access/mod.rs"]
pub mod git_access;
#[path = "../../../src/git_workspace.rs"]
pub mod git_workspace;
/// The checkout the engine runs from, found as it runs (decision 0224).
#[path = "../../../src/source_tree.rs"]
pub mod source_tree;
/// A regex's literal probe (the prefilter's; the store's vocabulary uses it too).
#[path = "../../../src/code_rule_dsl/prefilter/probes.rs"]
pub mod probes;
