//! What the content store observes, over its base (rerail-store-base): the observer of the files a run reads
//! (`content_store::observe`: their kinds, stats, digests and listings, through the journal), and every in-process
//! git read (`git_access`, the one opener and its pool; `git_workspace`, a repository's workspace, and its
//! snapshot, which the observer reads as files). Below the store (rerail-store), which re-exports it.
//!
//! The sources are the engine's, in place (`native/src`): only the build moved.

// The sources name the base's trait as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate rerail_store_base as rerail_store;

pub use rerail_store_base::{Canonical, probes, source_tree};

#[path = "../../../src/content_store"]
pub mod content_store {
    pub use rerail_store_base::content_store::*;
    pub mod observe;
}
/// Every in-process Git read: the one opener, its setup and its pool (`native/src/git_access`). Its Node-API
/// owner (`owner.rs`) is the engine's, which installs the setup.
#[path = "../../../src/git_access/mod.rs"]
pub mod git_access;
#[path = "../../../src/git_workspace.rs"]
pub mod git_workspace;
