//! The findings layer (docs/final-shape/03-architecture.md §8): finding identity and coverage,
//! the typed SQL shape that lowers to it, and S's oracle as a test.
pub(in crate::code_rule_dsl) mod contract;
pub(in crate::code_rule_dsl) mod shape;
// S's oracle for the async-call family, kept as a test only (05 Phase 8 row 5).
#[cfg(test)]
#[path = "findings/tests/async_reference.rs"]
mod async_reference;
