//! A lean Kotlin parser for the tape alone: tree-sitter-kotlin's tape, as the engine encodes its
//! trees (named-only, decision 0140's writer), written as the parse goes. The same approach as
//! `rerail-java-lean` and `rerail-rust-lean`: it stands in for tree-sitter where a file parses; a
//! file it cannot read exactly is left to tree-sitter (`Refusal`), which also recovers from errors.
//! There is one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`) gives the lexer its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The lexer** (`lex`) reads a token where the parser asks; every word is a name with a word
//!   code from one hash probe, and the parser decides where it is a keyword (tree-sitter's keyword
//!   extraction). tree-sitter-kotlin's external scanner is ported where the parser asks for it: the
//!   automatic semicolon (`Lexer::asi`, on the bytes between two tokens), a string's pieces and
//!   nested block comments.
//! - **The parser** (`parse`) is recursive descent, writing each node in preorder as it opens, with
//!   its code (kind, field); a node found after its first child (an operator after its left
//!   operand) is put before it (`Writer::insert`). Only the tokens tree-sitter's tape stores are
//!   written: the engine's implied-token automata say which (`kinds::DROPPED`).
//! - **Extras** (comments) are written when the parse next opens a node or reads a token, at the
//!   node then open.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS`, a set of
//! supertypes from `CLASSES` (tree-sitter-kotlin has none), and the extra flag. The engine maps each
//! to its grammar's ids by name.

mod block;
mod kinds;
mod lex;
mod parse;
mod tw;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS, f};
pub use lex::Refusal;
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0631).
pub type Tape = rerail_tape_format::f16::Finished;

/// The language's tape tag.
pub const TAPE_TAG: u8 = 24;

/// The line terminators of the language's ecosystem (decision 0612).
pub const TERMINATORS: rerail_tape_format::newlines::Terminators = rerail_tape_format::newlines::Terminators::Newlines;

/// Key `key`'s names, as the grammar names them: what its number in its language's table is found by (decision
/// 0631; `sink`, and the engine's `formats::lean_named`). `None` for a key the parser does not write.
pub fn names(key: u32) -> Option<rerail_tape_format::KeyNames> {
    let (kind, field, class, extra) = unkey(key);
    let (kind, named) = *KINDS.get(usize::from(kind))?;
    Some(rerail_tape_format::KeyNames {
        kind,
        named,
        field: FIELDS.get(usize::from(field)).copied()?,
        flags: if extra { rerail_tape_format::CODE_EXTRA } else { 0 },
        classes: if extra { &[] } else { CLASSES.get(usize::from(class)).copied()? },
    })
}

/// The crate's own sink (decision 0631): format 16, each key numbered in the language's table by its names (`names`),
/// as the engine numbers the parser's keys (its `formats::lean_named`): one numbering, so the tapes are the same.
pub fn sink() -> rerail_tape_format::Named {
    rerail_tape_format::Named::new(TAPE_TAG, TERMINATORS, names)
}

/// `sink`'s tape: its tree is the language's (a code its table lacks is the parser's bug).
pub(crate) fn written(t: Result<Tape, String>) -> Tape {
    t.unwrap_or_else(|e| panic!("the parser's tree is no format 16 tape of its language's table: {e}"))
}
pub use tw::key;

/// A key's kind, field, supertype set and extra flag (`key`'s inverse, in the other lean parsers'
/// form; tree-sitter-kotlin has no supertypes).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    ((key & 0x3ff) as u16, ((key >> 10) & 0x3f) as u8, 0, key >> 16 & 1 == 1)
}

/// The supertype sets a node is in, as the tape names them: tree-sitter-kotlin has none.
pub const CLASSES: &[&[&str]] = &[&[]];

/// The tape of `src`: tree-sitter-kotlin's, as the engine encodes it. `Err` where the parser
/// leaves the file to tree-sitter (a syntax error, a construct it does not read exactly).
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src)
}

/// [`tape`], the parser's writer writing to `finish`'s target and finished by it: format 16 or 16z, written by its ops
/// as the parse goes (`Writer::finish_16`, decision 0434).
pub fn tape_into<T>(src: &[u8], finish: &mut dyn rerail_tape_format::Sink<T>) -> Result<T, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_into(src, finish)
}

/// A tape as plain text, as the engine's `python_tape_plain` writes one (the oracle's form): a node
/// a line, indented by depth, its kind, field, flags, supertypes and span.
pub fn tape_text(tape: &Tape) -> String {
    tape.tape.text().unwrap_or_else(|e| panic!("a tape this crate wrote: {e}"))
}

/// The lexer alone over `src` (for measuring): the tokens it reads to the end, or to a refusal.
#[doc(hidden)]
pub fn lex_only(src: &[u8]) -> usize {
    let lx = lex::Lexer::new(src);
    let (mut extras, mut toks) = (Vec::new(), Vec::with_capacity(src.len() / 4));
    let mut at = 0;
    while let Ok(next) = lx.lex_run(at, &mut toks, 16, &mut extras) {
        if toks.last().is_some_and(|t| t.kind == lex::T::Eof) {
            return toks.len() - 1;
        }
        at = next;
    }
    toks.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_fit_the_keys() {
        assert!(KINDS.len() < 1024 && FIELDS.len() <= 64);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Identifier, f::RECEIVER, false)), (K::Identifier as u16, f::RECEIVER, 0, false));
        assert_eq!(unkey(key(K::LineComment, 0, true)), (K::LineComment as u16, 0, 0, true));
    }
}
