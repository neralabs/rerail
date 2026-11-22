//! The tree watch (`native/src/tree_watch/`), mounted in place as the crate split's crates mount theirs, with the panic
//! boundary its OS threads report through (`native/src/panic_boundary.rs`, rerail-workspace's too: a function, no state).
#[path = "../../../src/panic_boundary.rs"]
pub mod panic_boundary;
#[path = "../../../src/tree_watch/mod.rs"]
pub mod tree_watch;
