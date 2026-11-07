//! Planner statistics (03 §6, §16; 05 Phase 3 rows 6–8): plan inputs read from the content store's
//! SQL-owned summaries in one batch (`inputs`), the summaries ingestion maintains (`ingest`), and
//! compiled plans with their validity, feedback and SQLite statistics (`plans`, over the bounded
//! cache in `cache`).
pub(in crate::code_rule_dsl) mod cache;
pub(in crate::code_rule_dsl) mod identity;
pub(in crate::code_rule_dsl) mod ingest;
pub(in crate::code_rule_dsl) mod inputs;
pub(in crate::code_rule_dsl) mod plans;
