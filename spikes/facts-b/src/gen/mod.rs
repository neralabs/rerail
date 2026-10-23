//! The delta-plan generator (Phase 2 deliverable 4, invariant I8): a repository plan in the IR
//! (`rerail_facts::ir::Plan<RepoSource>`) is normalised to delta operators (`norm`) and lowered to
//! one differential dataflow over the membership model's tagged records (`lower`). No plan-specific
//! delta code exists; Spike B's hand expansions (`crate::flow`) are the generator's test vectors
//! (`vectors`). M2's hybrid model (`hybrid`) runs the same normalised plans from scratch on a
//! set-at-a-time executor (`batch`) over resident per-content exports.
pub mod batch;
pub mod declared;
pub mod drive;
pub mod hybrid;
pub mod lower;
pub mod norm;
pub mod prof;
pub mod real;
pub mod scalar;
pub mod tags;
pub mod val;
pub mod vectors;
