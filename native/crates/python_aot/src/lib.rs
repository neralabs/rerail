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

pub use tier1::{DERIVED, Lexed, Rec, SPANNED, carried, code_at, derived, lex, spanned};
/// The supertypes a key's class bits name (`rerail_tape_format::Writer::classes`), ascending.
pub use tier2::SUPERS;

/// `src`'s tape, written to `finish`'s target and finished by it (decision 0434), or the byte the parser refused it at
/// (0: tier 1 sends it to tree-sitter). Each code's key is the parser's (`unkey`), which the sink numbers by its names
/// (decision 0631).
pub fn parse<F: rerail_tape_format::Finish>(src: &[u8], finish: &mut F) -> Result<F::Out, u32> {
    tier2::parse(src, &tier1::lex(src), finish)
}

/// A key's kind and field (the grammar's ids), its flags (`CODE_EXTRA`), and the supertypes its class bits name
/// (bit `CLASS_SHIFT + j`: `SUPERS[j]`), ascending.
pub fn unkey(key: u32) -> (u16, u16, u8, impl Iterator<Item = u16>) {
    use rerail_tape_format::{CLASS_SHIFT, KIND_BITS};
    let classes = (0..SUPERS.len()).filter(move |&j| key >> (CLASS_SHIFT + j as u32) & 1 != 0).map(|j| SUPERS[j]);
    ((key & ((1 << KIND_BITS) - 1)) as u16, (key >> KIND_BITS & 0x3f) as u16, (key >> 16 & 0x7) as u8, classes)
}
