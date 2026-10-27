//! Batch admission's measurements, from Pulling-it-all-together: process residency samples
//! (`profile/memory.rs`). P's payload census (`workspace/memory/census.rs`) needs P's typed
//! projections (`violations/projection.rs`), and P's admission loop (`relational/batch.rs`) needs
//! D's session machinery; both stay outside the module tree.
pub(crate) mod memory;
