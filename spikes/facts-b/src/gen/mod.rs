//! The delta-plan generator (Phase 2 deliverable 4, invariant I8): a repository plan in the IR
//! (`rerail_facts::ir::Plan<RepoSource>`) is normalised to delta operators (`norm`) and lowered to
//! one differential dataflow over the membership model's tagged records (`lower`). No plan-specific
//! delta code exists; Spike B's hand expansions (`crate::flow`) are the generator's test vectors
//! (`vectors`).
pub mod drive;
pub mod lower;
pub mod norm;
pub mod prof;
pub mod real;
pub mod scalar;
pub mod tags;
pub mod val;
pub mod vectors;
pub mod y2dump;
