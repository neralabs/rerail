//! The `.rerail` parser as Rust, written by the Bitlex compiler from the grammar (`rr_parser/rr.bitlex`), never by
//! hand: tier 1 (`tier1.rs`) from the lexer's statements (`rerail_compiler::bitlex_lex_rust`). It is the compile's
//! that `rerail_rr_parser::compiled` makes, so its codes are the JIT's. `rerail-rr-parser`'s `rust_is_current`
//! checks it is the compiler's output, and writes it again with `RERAIL_BLESS=1`; its tests check tier 1's records
//! against the interpreter's.
//!
//! This replaces the crate's lowerings of the JIT's IR (`clif_rust`, `rust_stream`, decision 0550).

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables, unused_assignments)]
mod tier1;

pub use tier1::{Lexed, lex};
