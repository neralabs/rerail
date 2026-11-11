//! Batch admission's measurements, from Pulling-it-all-together: process residency samples
//! (`profile/memory.rs`), and P's payload census (`workspace/memory/census.rs`) over W's JSON
//! projections. The relational module also provides the compatibility contract for recorded
//! data-layer policies. It uses the production matcher and does not restore the retired anchor corpus.
pub mod census;
pub mod memory;
#[allow(dead_code)]
pub mod relational;
