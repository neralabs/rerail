//! Planner statistics (03 §6, §16; 05 Phase 3 rows 6–8): plan inputs read from the content store's
//! SQL-owned summaries in one batch (`inputs`), the summaries ingestion maintains (`ingest`), and
//! compiled plans with their validity, feedback and SQLite statistics (`plans`, over the bounded
//! cache in `cache`).
pub mod cache;
pub use rerail_facts::code_rule_dsl::stats::identity;
pub mod ingest;
pub mod inputs;
pub mod plans;
