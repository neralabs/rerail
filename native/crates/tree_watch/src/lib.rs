//! The tree watch (`src/tree_watch/`, the crate split moved it here in step 9), with the panic
//! boundary its OS threads report through (`native/crates/tree_watch/src/panic_boundary.rs`, rerail-workspace's too: a function, no state).
#[path = "panic_boundary.rs"]
pub mod panic_boundary;
#[path = "tree_watch/mod.rs"]
pub mod tree_watch;
