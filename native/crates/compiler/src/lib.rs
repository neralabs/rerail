//! The transducer's parser compiler's front end: the programs and what is known of them, which every backend reads.
//! The grammar's state machine (`grammar`); tier 1's block programs, their interpreter (`block`), their native IR
//! (`block_native`) and its analyses (`lane_plan`, `byte_needs`, `native_cpu_edge`, `native_cpu_pairs`); tier 2's
//! words and tables (`stream_ir`, `rir_text`, `rir_asm`, `stream`, `native_ir`); the C preprocessor's words
//! (`pp_words`); and what every JIT function shares (`native_cpu_codegen`, the shape it must have, `native_cpu_shape`,
//! and its register pressure, `native_cpu_pressure`).
//!
//! The compilers and backends are the crates above, each its own so that an edit to one rebuilds it and what reads
//! it, not the front end's readers: the Bitlex compiler (`rerail-bitlex`), tier 1 on the CPU (`rerail-cpu-block`),
//! tier 2 on the CPU (`rerail-cpu-stream`), both tiers as Rust source (`rerail-rust-stream`), the C preprocessor on
//! the CPU (`rerail-cpu-pp`) and Metal's (`rerail-metal`). Their sources stay in this folder; they mount them. The engine
//! (`code_rule_dsl::transducer::packed`) re-exports each module under the same name.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

pub mod block;
pub mod class_pack;
pub mod block_native;
pub mod clif_fold;
pub mod clif_text;
pub mod grammar;
mod error;
pub mod native_cpu_codegen;
pub mod native_cpu_edge;
/// Which of tier 1's values are byte-local (`Plan`), for every vector backend.
pub mod lane_plan;
/// What tier 1's outputs need of the bytes, and which are made from them alone.
pub mod byte_needs;
pub mod native_cpu_pairs;
pub mod native_cpu_pressure;
pub mod native_cpu_shape;
pub mod native_ir;
pub mod program;
// The RIR assembler and the preprocessor's words (`rir_asm`, `pp_words`, `pp_expr`) are rerail-rir's, over this
// crate (step 7a).
pub mod records;
pub mod rir_text;
pub mod set_mode;
pub mod stream;
/// Tier 1's record tables, as the token loop reads them.
pub mod record_tables;
pub mod stream_ir;
/// Which tier 1 values are token-level checks (doc 25): local windows, anchors, readers.
pub mod token_local;

pub use error::Error;

#[cfg(test)]
mod grammar_tests;
