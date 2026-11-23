//! The engine's content store (`content_store`): sources, tapes, memos and run records in SQLite, the
//! `ContentStore` and every module that writes and reads through it. Over its base (rerail-store-base) and its
//! observer and git (rerail-observe), which it re-exports at their old paths (`content_store::*`, `git_access`,
//! `git_workspace`), so no caller changed.
//!
//! The sources are the engine's, in this crate's `src/content_store` (the crate split, step 9). What the store needs
//! of the engine, the engine hands it as it loads: the SQL functions every connection gets
//! ([`content_store::set_connection_functions`]).

// The store's files name the base's trait as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate self as rerail_store;

pub use rerail_observe::{Canonical, git_access, git_workspace, probes, source_tree};
/// Git and worktree discovery's tests (rerail-observe's `git_workspace`): they publish to the store.
#[cfg(test)]
#[path = "git_workspace_tests.rs"]
mod git_workspace_tests;

#[path = "content_store/mod.rs"]
pub mod content_store;

/// This crate's own test binary, which boot does not load: the store is registered with the observer and git below it
/// (`content_store::observer_store::STORE`) as the binary loads, as boot registers it (`install_with`).
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::content_store::observer_store::register();
        // Blocks that start small, so that its stores grow chains and roll them up (`rerail_atoms::small_blocks`).
        rerail_atoms::set_small_blocks();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
