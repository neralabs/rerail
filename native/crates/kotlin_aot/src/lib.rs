//! Kotlin's parser as Rust, written by the Bitlex compiler from the grammar, never by hand: tier 1
//! (`tier1.rs`) from the lexer's statements (`rerail_bitlex::bitlex_lex_rust`), tier 2
//! (`tier2.rs`) from the tree rules (`rerail_bitlex::bitlex_rust`).
//! `rerail-kotlin-parser`'s `rust_is_current` test checks they are the compiler's output, and
//! writes them again with `RERAIL_BLESS=1`. Tier 1 reads the bytes and writes the records (each
//! token's first and last byte and its code); tier 2 reads them and writes the tape.
//! `rerail-kotlin-parser`'s tests read every text they check through it too, against the lean
//! parser's tape, and tier 1's records against the interpreter's.

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier1;

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier2;

pub use tier1::{Lexed, lex};

/// `src`'s tape, or the byte the parser refused it at (0: tier 1 sends it to tree-sitter).
/// The columns, as the lean parser finishes them (`kotlin_lean::Tape`).
pub fn parse(src: &[u8]) -> Result<rerail_tape_format::Columns, u32> {
    tier2::parse(src, &tier1::lex(src), &mut Columns)
}

/// The writer finished into its columns.
struct Columns;

impl rerail_tape_format::Finish for Columns {
    type Out = rerail_tape_format::Columns;
    fn finish<L: rerail_tape_format::Layout>(&mut self, w: rerail_tape_format::Writer<L>) -> Result<rerail_tape_format::Columns, rerail_tape_format::Redo> {
        w.finish_columns()
    }
}
