//! JSON's parser in simdjson's structure as Rust (`json_parser/json-simdjson.bitlex`), written by the Bitlex
//! compiler from the grammar, never by hand: tier 1 (`tier1.rs`) from its lexer statements
//! (`rerail_compiler::bitlex_lex_rust`, simdjson's stage 1: each token's first byte), tier 2 (`tier2.rs`) from its
//! tree rules in one loop (`rerail_compiler::bitlex_rust::tier2_loop`, simdjson's stage 2), writing format 16.
//! `rerail-parser-tests`' `json_rust_is_current` checks they are the compiler's output, and writes them again with
//! `RERAIL_BLESS=1`; its tests check the tapes against the JIT's.

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier1;

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables, unused_assignments)]
mod tier2;

pub use tier1::{Lexed, lex};
/// The code table tier 2's numbers are of: each number's kind, field and supertypes.
pub use tier2::{ENTRIES, WIDTHS};

/// `src`'s tape into `w` (a format 16 writer of a language whose table is `ENTRIES`), or the byte the parser
/// refused it at (0: tier 1 sends it to tree-sitter).
pub fn parse(src: &[u8], w: &mut rerail_tape_format::f16::Writer) -> Result<(), u32> {
    tier2::parse(src, &tier1::lex(src), w)
}
