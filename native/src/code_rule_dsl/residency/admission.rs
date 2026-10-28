//! Batch admission's measurements, from Pulling-it-all-together: process residency samples
//! (`profile/memory.rs`), and P's payload census (`workspace/memory/census.rs`) over W's JSON
//! projections. P's admission loop (`relational/batch.rs`) runs P's relational stage, placed with it
//! as a test-only module (`relational`): its clauses carry SQL, which rules do not (open item O18).
pub(crate) mod census;
pub(crate) mod memory;
#[cfg(test)]
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod relational;
