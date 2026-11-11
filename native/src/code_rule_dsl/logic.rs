//! The logic language (docs/final-shape/03-architecture.md §12; 05 Phases 5–6): rules as named
//! facts and Datalog over them, planned from statistics and lowered to SQL or transducer groups.
//! `sql` is the typed SQL its plans emit.

pub(in crate::code_rule_dsl) mod crypto;
pub(in crate::code_rule_dsl) mod derived;
#[cfg(test)]
mod facts_only;
pub(in crate::code_rule_dsl) mod analysis;
// Nothing emits SQL through them until the planner (Phase 6) does.
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod front;
pub(in crate::code_rule_dsl) mod costs;
// The logic's device interface; the engine registers a device as it loads (`crate::install_devices`).
pub(in crate::code_rule_dsl) mod device;
pub(in crate::code_rule_dsl) mod lanes;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod answers;
pub(in crate::code_rule_dsl) mod liveness;
pub(in crate::code_rule_dsl) mod magic;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod native;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod plan;
pub(in crate::code_rule_dsl) mod profile;
pub(in crate::code_rule_dsl) mod program_facts;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod rr;
pub(crate) mod settings;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod sql;
pub(in crate::code_rule_dsl) mod simplify;
pub(in crate::code_rule_dsl) mod strings;
pub(in crate::code_rule_dsl) mod convert;
