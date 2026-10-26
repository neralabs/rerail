//! The fact engine above the content store (docs/final-shape/03-architecture.md §1, §2, §14).

// The push path is wired into the workspace scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod memo;
#[allow(dead_code)]
pub(crate) mod observe;
#[allow(dead_code)]
pub(crate) mod propagate;
#[allow(dead_code)]
pub(crate) mod runs;
