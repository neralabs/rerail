//! The engine's content store (`content_store`): sources, tapes, memos and run records in SQLite, the
//! `ContentStore` and every module that writes and reads through it, with the observer and every in-process git read,
//! which read the store (the observer's answers are its kept facts, t33 K2). Over its base (rerail-store-base),
//! which it re-exports at its old paths (`content_store::*`), so no caller changed.
//!
//! The sources are the engine's, in place (`native/src/content_store`): only the build moved. What the store needs
//! of the engine, the engine hands it as it loads: the SQL functions every connection gets
//! ([`content_store::set_connection_functions`]).

// The store's files name the base's trait as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate self as rerail_store;

pub use rerail_store_base::{Canonical, probes, source_tree};
/// Every in-process Git read: the one opener, its setup and its pool (`native/src/git_access`). Its Node-API
/// owner (`owner.rs`) is the engine's, which installs the setup.
#[path = "../../../src/git_access/mod.rs"]
pub mod git_access;
/// A repository's workspace and its snapshot, which the observer reads as files.
#[path = "../../../src/git_workspace.rs"]
pub mod git_workspace;
/// Git and worktree discovery's tests (`git_workspace`'s): they publish to the store.
#[cfg(test)]
#[path = "../../../src/git_workspace_tests.rs"]
mod git_workspace_tests;

#[path = "../../../src/content_store/mod.rs"]
pub mod content_store;

// The crate's own tests make their folders the user's alone, as the engine's test binaries do (decision 0574): a
// host's umask of `002` made a fixture's folder `775`, which the mapped files' rules refuse.
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;
