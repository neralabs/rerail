//! The `demand(batch)` relation (docs/final-shape/03-architecture.md §3; 05 Phase 5): SQL names a
//! batch of (program group, tapes) and the transducer runs once for it in `xFilter`; the rows are
//! the group's matches and facts.

// Nothing registers a batch until the logic planner (Phase 6) and the waves (Phase 7) do.
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod driver;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod vtab;
