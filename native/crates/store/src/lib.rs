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
