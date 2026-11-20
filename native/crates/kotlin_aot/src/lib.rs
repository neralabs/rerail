//! Kotlin's tier 2 as Rust: `tier2.rs`, written by the Bitlex compiler from the grammar's tree
//! rules (`rerail_compiler::bitlex_rust`, `rerail_kotlin_parser::tier2_rust`), never by hand.
//! `rerail-kotlin-parser`'s `tier2_rust_is_current` test checks it is the compiler's output, and
//! writes it again with `RERAIL_BLESS=1`. It reads tier 1's records (each token's first and last
//! byte and its code) and writes the tape. `rerail-kotlin-parser`'s tests read every text they
//! check through it too, against the lean parser's tape.

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier2;

pub use tier2::parse;
