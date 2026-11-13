//! The findings layer (docs/final-shape/03-architecture.md §8): finding identity and coverage,
//! the typed SQL shape that lowers to it, and S's oracle as a test.
pub mod contract;
pub mod shape;
// S's oracle for the async-call family, kept as a test only (05 Phase 8 row 5).
// cratify: `async_reference` is rerail-lang-python's (it mounts it)
