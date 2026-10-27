//! Planner statistics (03 §6, §16; 05 Phase 3 rows 6–8): plan inputs read from the content store's
//! SQL-owned summaries in one batch (`inputs`), and a bounded cache of compiled SQL (`cache`).
pub(in crate::code_rule_dsl) mod cache;
pub(in crate::code_rule_dsl) mod inputs;
