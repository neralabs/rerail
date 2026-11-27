//! Python's parser as Rust, written by the Bitlex compiler from the grammar, never by hand: tier 1
//! (`tier1.rs`) from the lexer's statements (`rerail_bitlex::bitlex_lex_rust`), tier 2
//! (`tier2.rs`) from the tree rules (`rerail_bitlex::bitlex_rust`).
//! `rerail-python-parser`'s `rust_is_current` test checks they are the compiler's output, and
//! writes them again with `RERAIL_BLESS=1`. Tier 1 reads the bytes and writes the records (each
//! token's first and last byte and its code); tier 2 reads them and writes the tape.
//! `rerail-python-parser`'s tests read every text they check through it too, against the lean
//! parser's tape, and tier 1's records against the interpreter's.

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier1;

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier2;

pub use tier1::{Lexed, PADDING, UNRECORDED, lex};
/// The supertypes a key's class bits name (bit `rerail_tape_format::CLASS_SHIFT + j` is `SUPERS[j]`), ascending.
pub use tier2::SUPERS;

/// The tape of `buf[..n]`, written to `finish`'s target and finished by it (decision 0434), or the byte the parser
/// refused it at (0: tier 1 sends it to tree-sitter). `buf` is the file padded, as simdjson's: `PADDING` spaces after
/// it (`padded`), so that tier 1 loads each block where it is. Each code's key is the tape format's
/// (`kf | flags << 16`), its supertypes' bits from `CLASS_SHIFT` (`SUPERS`), not the lean parser's own: `finish`
/// numbers them.
pub fn parse<F: rerail_tape_format::Finish>(buf: &[u8], n: usize, finish: &mut F) -> Result<F::Out, u32> {
    tier2::parse(&buf[..n], &tier1::lex(buf, n), finish)
}

/// `src` padded for `parse`: a buffer of it and `PADDING` spaces (a caller that reads files makes its buffer so).
pub fn padded(src: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(src.len() + PADDING);
    b.extend_from_slice(src);
    b.resize(src.len() + PADDING, b' ');
    b
}
