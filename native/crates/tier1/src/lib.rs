//! Tier 1 of the transducer's parsers, in a crate of its own so that a change to it rebuilds
//! only this and a link: block programs (`block`, with their interpreter), their native IR
//! (`block_native`), and their Cranelift lowering (`native_cpu_block`, `native_cpu_lanes`), the
//! shape every generated function must have (`native_cpu_shape`), and its register pressure
//! (`native_cpu_pressure`). The engine (`code_rule_dsl::transducer::packed`) re-exports each
//! under the same name.

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

pub mod block;
pub mod block_native;
mod error;
pub mod native_cpu_block;
pub mod native_cpu_codegen;
pub mod native_cpu_lanes;
pub mod native_cpu_pressure;
pub mod native_cpu_shape;

pub use error::Error;
