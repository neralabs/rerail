//! The engine's content store and the git workspace it reads, as a crate of their own.
//!
//! The sources are the engine's, in place (`native/src/content_store`, `native/src/git_workspace.rs`): only
//! the build moved. The engine re-exports both modules at their old paths (`crate::content_store`,
//! `crate::git_workspace`), so no caller changed. What the store needs of the engine, the engine hands it as
//! it loads: the SQL functions every connection gets ([`content_store::set_connection_functions`]).

#[path = "../../../src/content_store/mod.rs"]
pub mod content_store;
/// The Git opener the workspace discovery and the ignore matcher read through, its setup and its pool
/// (`native/src/git_access`). Its Node-API owner (`owner.rs`) is the engine's, which installs the setup.
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
