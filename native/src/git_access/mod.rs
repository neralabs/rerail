//! Every in-process Git read of the engine goes through this module, so the engine answers as the
//! Git Rerail runs does, or gives no answer.
//!
//! - [`opener`]: the one opener. It refuses where Git refuses, names the config files Rerail's
//!   Git reads, and never lets gix start a Git of its own to find them.
//! - [`setup`]: what Rerail's Git reported, installed once per process.
//! - [`pool`]: the four threads Git reads run on, each read under a time limit, a panic caught.
//! - [`owner`]: the one Node-API object per process that installs the setup and hands out readers.
//!
//! The ignore matcher (`git_ignore.rs`) and the workspace discovery (`git_workspace.rs`) open
//! their repositories here.

pub(crate) mod opener;
pub mod owner;
pub(crate) mod pool;
pub(crate) mod setup;

#[cfg(test)]
mod tests;
