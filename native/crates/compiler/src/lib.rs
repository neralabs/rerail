//! The transducer's parser compiler, in a crate of its own so that a change to it rebuilds
//! only this and a link. A Bitlex file compiles here end to end: its masks and tokens to a
//! block program (`bitlex_masks`, `bitlex_prove`), its tree section to the grammar's state
//! machine (`grammar`) and to tier 2's words and tables (`bitlex_tier2`, `stream_ir`,
//! `rir_text`). Tier 1's block programs have their interpreter (`block`), their native IR
//! (`block_native`) and their Cranelift lowering (`native_cpu_block`, `native_cpu_lanes`),
//! with the shape every generated function must have (`native_cpu_shape`) and its register
//! pressure (`native_cpu_pressure`). The engine (`code_rule_dsl::transducer::packed`)
//! re-exports each under the same name.

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

pub mod bitlex_masks;
pub mod bitlex_prove;
pub mod bitlex_tier2;
pub mod block;
pub mod block_native;
pub mod clif_text;
pub mod grammar;
/// The Bitlex ladder's checks, for this crate's tests and the engine's.
#[cfg(any(test, feature = "test-hooks"))]
pub mod ladder;
mod error;
pub mod native_cpu_block;
pub mod native_cpu_codegen;
pub mod native_cpu_lanes;
pub mod native_cpu_pressure;
// Row programs' Cranelift definer (`block::BlockProgram::rows`, decision 0095 §2).
pub mod native_cpu_rows;
pub mod native_cpu_clusters;
pub mod native_cpu_shape;
pub mod native_cpu_stream;
pub mod native_ir;
pub mod program;
pub mod records;
pub mod rir_asm;
pub mod rir_text;
pub mod set_mode;
pub mod stream;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod records_tests;
pub mod stream_ir;
/// Which tier 1 values are token-level checks (doc 25): local windows, anchors, readers.
pub mod token_local;
#[cfg(test)]
mod token_local_tests;
#[cfg(test)]
mod remade_tests;
#[cfg(test)]
mod totals_tests;

pub use error::Error;

#[cfg(test)]
mod delay_tests;
#[cfg(test)]
mod gating_tests;
#[cfg(test)]
mod outline_tests;
#[cfg(test)]
mod tier2_tests;
#[cfg(test)]
mod ladder_tests;
#[cfg(test)]
mod grammar_tests;
