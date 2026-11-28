//! JSON's parser in simdjson's structure as Rust (`json_parser/json-simdjson.bitlex`), written by the Bitlex
//! compiler from the grammar, never by hand: tier 1 (`tier1.rs`) from its lexer statements
//! (`rerail_bitlex::bitlex_lex_rust`, simdjson's stage 1: each token's first byte), tier 2 (`tier2.rs`) from its
//! tree rules in one loop (`rerail_bitlex::bitlex_rust::tier2_loop`, simdjson's stage 2), writing format 16.
//! `rerail-parser-tests`' `json_rust_is_current` checks they are the compiler's output, and writes them again with
//! `RERAIL_BLESS=1`; its tests check the tapes against the JIT's.

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables, unused_assignments)]
mod tier1;

#[rustfmt::skip]
#[allow(clippy::all, unused_parens, unused_mut, dead_code, unreachable_code, unused_variables, unused_assignments)]
mod tier2;

pub use tier1::{Carries, LANES, Lexed, PADDING, TYPE_NAMES, TYPES, Typed, entries, entries_in, lane_carries, lane_of, lex, lex_chunked, lex_typed, lex_typed_in, stage1_ends_best, stage1_from_best};
/// The code table tier 2's numbers are of: each number's kind, field and supertypes.
pub use tier2::{ENTRIES, WIDTHS};
/// Tier 1's passes as UKL, printed from its Rust (decision 0726; `bitlex_lex_rust::ukl`, held by `json_rust_is_current`):
/// kernels `json_t1_ends` and `json_t1_from`.
pub const TIER1_UKL: &str = include_str!("tier1.uglykl");
/// Tier 2's passes as UKL, printed from its Rust (decision 0726): kernels `json_t2_tokens` and `json_t2_write`.
pub const TIER2_UKL: &str = include_str!("tier2.uglykl");

/// Tier 2 from the middle (decision 0726): the walk from any place, and the three passes over chunks of records.
pub mod middle {
    pub use crate::tier2::{At, CYCLES, Entry, Frame, Lane, NONE, NO_START, Out, RETS, Refusal, SEPS, Shadow, Stop, Summary, TOKEN_FIRSTS, Token, Touch, apply, chunk_starts, compose, depths, each, entries, in_order, lanes, parse_middle, run_range, scan, serial_entries, summaries, token_at, tokens, tree, write_range, Order};
}

/// The tape of `buf[..n]` into `w` (a format 16 writer of a language whose table is `ENTRIES`), or the byte the parser
/// refused it at (0: tier 1 sends it to tree-sitter). `buf` is the file padded, as simdjson's: `PADDING` spaces after
/// it, so that tier 1 loads each block where it is and tier 2 reads past the last record unchecked.
pub fn parse(buf: &[u8], n: usize, w: &mut rerail_tape_format::f16::Writer) -> Result<(), u32> {
    parse_lexed(buf, &tier1::lex(buf, n), w)
}

/// `parse`'s tier 2 alone, on tier 1's records of `buf` (`lex`).
pub fn parse_lexed(buf: &[u8], lexed: &Lexed, w: &mut rerail_tape_format::f16::Writer) -> Result<(), u32> {
    tier2::parse(buf, lexed, w)
}

/// `src` padded for `parse`: a buffer of it and `PADDING` spaces (a caller that reads files makes its buffer so).
pub fn padded(src: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(src.len() + PADDING);
    b.extend_from_slice(src);
    b.resize(src.len() + PADDING, b' ');
    b
}

#[cfg(test)]
mod tests;
