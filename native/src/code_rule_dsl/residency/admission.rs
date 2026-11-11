//! Batch admission's measurements, from Pulling-it-all-together: process residency samples
//! (`profile/memory.rs`), and P's payload census (`workspace/memory/census.rs`) over W's JSON
//! projections. The relational module also provides the compatibility contract for recorded
//! data-layer policies. It uses the production matcher and does not restore the retired anchor corpus.
pub(crate) mod census;
pub(crate) mod memory;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod relational;
