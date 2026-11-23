//! The `.rerail` parser as Rust, written by the Bitlex compiler from the grammar (`rr_parser/rr.bitlex`), never by
//! hand: tier 1 (`tier1.rs`) from the lexer's statements (`rerail_bitlex::bitlex_lex_rust`), tier 2 (`tier2.rs`) from
//! the tree rules as one loop over tier 1's records (`rerail_bitlex::bitlex_rust`'s `tier2_loop`, decision 0618). It is
//! the compile's that `rerail_rr_parser::compiled` makes, so its codes are the JIT's. `rerail-rr-parser`'s `rust_is_current`
//! checks they are the compiler's output, and writes them again with `RERAIL_BLESS=1`; its tests check tier 1's
//! records against the interpreter's, and the tapes against the JIT's.
//!
//! This replaces the crate's lowerings of the JIT's IR (`clif_rust`, `rust_stream`, decision 0550).

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables, unused_assignments)]
mod tier1;

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables)]
mod tier2;

pub use tier1::{Lexed, PADDING, lex};

/// The tape of `buf[..n]`, format 15's image as the lean parser finishes it (`Writer::finish`), or the byte the parser
/// refused it at (0: tier 1 sends it to the engine). `buf` is the file padded, as simdjson's: `PADDING` spaces after it
/// (`padded`), so that tier 1 loads each block where it is.
pub fn parse(buf: &[u8], n: usize) -> Result<Vec<u8>, u32> {
    tier2::parse(&buf[..n], &tier1::lex(buf, n), &mut Image(&buf[..n]))
}

/// The writer finished into the image of the source it read.
struct Image<'s>(&'s [u8]);

impl rerail_tape_format::Finish for Image<'_> {
    type Out = Vec<u8>;
    fn finish<L: rerail_tape_format::Layout>(&mut self, w: rerail_tape_format::Writer<L>) -> Result<Vec<u8>, rerail_tape_format::Redo> {
        w.finish(self.0)
    }
}

/// `src` padded for `parse` and `lex`: a buffer of it and `PADDING` spaces (a caller that reads files makes its buffer
/// so).
pub fn padded(src: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(src.len() + PADDING);
    b.extend_from_slice(src);
    b.resize(src.len() + PADDING, b' ');
    b
}
