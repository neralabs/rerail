//! The fact engine above the content store (docs/final-shape/03-architecture.md §1, §2, §14).

// The push path (observers, overlay, writer) is its first caller.
#[allow(dead_code)]
pub(crate) mod propagate;
