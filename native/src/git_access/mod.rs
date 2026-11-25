//! The engine's ignore matcher and workspace discovery read Git in process through this module, so
//! they answer as the Git Rerail runs does, or give no answer. Three reads of the code rules open
//! gix directly still ([`opener`] names them).
//!
//! - [`opener`]: the one opener. It refuses where Git refuses, names the config files Rerail's
//!   Git reads, and never lets gix start a Git of its own to find them.
//! - [`setup`]: what Rerail's Git reported, installed once per process.
//! - [`pool`]: each open of a repository on a thread of its own under a time limit, a panic
//!   caught. The store's walk and the ignore questions run on the caller's thread.
//! - `owner.rs`: the one Node-API object per process that installs the setup and hands out readers.
//!   The opener, setup and pool are built in the store's crate (`rerail-store`), which the workspace
//!   discovery and the ignore matcher reach; the owner is the engine's (`crate::git_access::owner`
//!   in `native/src/lib.rs`), where Node-API and the ignore matcher are.
//!
//! The ignore matcher (`git_ignore.rs`) and the workspace discovery (`git_workspace.rs`) open
//! their repositories here.

pub mod opener;
pub mod pool;
pub mod setup;

// The tests (`tests.rs`) are the engine's (`native/src/lib.rs`): they ask the ignore matcher too.
