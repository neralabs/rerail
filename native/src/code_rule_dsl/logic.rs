//! The logic language (docs/final-shape/03-architecture.md §12; 05 Phases 5–6): rules as named
//! facts and Datalog over them, planned from statistics and lowered to SQL or transducer groups.
//! `sql` is the typed SQL its plans emit.

// Nothing emits SQL through them until the planner (Phase 6) does.
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod front;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod magic;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod native;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod plan;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod sql;
