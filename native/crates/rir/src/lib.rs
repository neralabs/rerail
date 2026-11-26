//! The transducer's programs as text, over the compiler's front end (rerail-compiler): the RIR assembler
//! (`rir_asm`, which reads and writes a program's words) and the C preprocessor's words it validates (`pp_words`,
//! with their expressions, `pp_expr`). Below the CPU tiers, the parsers and the transducer, which assemble
//! programs with it.
//!
//! Its sources stay where they were (crates/compiler/src): this crate mounts them in place. It re-exports the
//! front end by glob, so they read it at the paths they did (`crate::program`, `crate::records`, `crate::Error`).
#![allow(clippy::type_complexity)]

pub use rerail_compiler::*;

#[path = "../../compiler/src/pp_expr.rs"]
pub mod pp_expr;
#[path = "../../compiler/src/pp_words.rs"]
pub mod pp_words;
#[cfg(test)]
#[path = "../../compiler/src/pp_words_tests.rs"]
mod pp_words_tests;
#[path = "../../compiler/src/rir_asm.rs"]
pub mod rir_asm;
