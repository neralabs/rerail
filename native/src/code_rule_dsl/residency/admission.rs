//! Batch admission's measurements, from Pulling-it-all-together: process residency samples
//! (`profile/memory.rs`), and P's payload census (`workspace/memory/census.rs`) over W's JSON
//! projections. P's admission loop (`relational/batch.rs`) needs D's session machinery and stays
//! outside the module tree.
pub(crate) mod census;
pub(crate) mod memory;
