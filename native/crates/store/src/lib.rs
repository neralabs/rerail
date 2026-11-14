//! The engine's content store and the git workspace it reads, as a crate of their own.
//!
//! The sources are the engine's, in place (`native/src/content_store`, `native/src/git_workspace.rs`): only
//! the build moved. The engine re-exports both modules at their old paths (`crate::content_store`,
//! `crate::git_workspace`), so no caller changed. What the store needs of the engine, the engine hands it as
//! it loads: the SQL functions every connection gets ([`content_store::set_connection_functions`]).

#[path = "../../../src/content_store/mod.rs"]
pub mod content_store;
#[path = "../../../src/git_workspace.rs"]
pub mod git_workspace;
/// A regex's literal probe (the prefilter's; the store's vocabulary uses it too).
#[path = "../../../src/code_rule_dsl/prefilter/probes.rs"]
pub mod probes;
