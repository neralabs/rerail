//! A lean C and C++ parser for the tape alone: tree-sitter-c's or tree-sitter-cpp's tape, as the
//! engine encodes its trees (named-only, decision 0140's writer), written as the parse goes. The same
//! approach as `rerail-go-lean` and `rerail-java-lean`: it stands in for tree-sitter where a file
//! parses; a file it cannot read exactly is left to tree-sitter (`Refusal`), which also recovers
//! from errors. There is one behavior: tree-sitter's tape, or a refusal.
//!
//! One parser reads both languages (`Dialect`): tree-sitter-cpp's grammar extends tree-sitter-c's,
//! and C's node kinds are among C++'s.
//!
//! - **Stage 1** (`block`) gives the lexer its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The lexer** (`lex`) reads a token where the parser asks; every word is a name with a word
//!   code from one probe of the keyword switch, and the parser decides where it is a keyword
//!   (tree-sitter's keyword extraction). Each token says where a `\n` before it is: a directive
//!   ends there.
//! - **The parser** (`parse`) is recursive descent, writing each node in preorder as it opens, with
//!   its code (kind, field, supertypes); a node found after its first child (an operator after its
//!   left operand) is put before it (`Writer::insert`). Only the tokens tree-sitter's tape stores
//!   are written: the engine's implied-token automata say which (`kinds::DROPPED_C`,
//!   `kinds::DROPPED_CPP`, and a gap's delimiters by its shape).
//! - **Extras** (comments) are written when the parse next opens a node or reads a token, at the
//!   node then open.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS`, a set of
//! supertypes from `CLASSES`, and the extra flag. The engine maps each to its grammar's ids by name.

// `lex::w`'s word list is one macro call a word deep.
#![recursion_limit = "512"]

mod block;
mod kinds;
mod lex;
mod parse;
pub mod pp;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS, f};
pub use lex::{NO_NL, Refusal, T, Tok, number_end, refused, word};
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0631).
pub type Tape = rerail_tape_format::f16::Finished;

/// Each dialect's tape tag.
pub const fn tape_tag(dialect: Dialect) -> u8 {
    match dialect {
        Dialect::C => 28, Dialect::Cpp => 29,
    }
}

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
pub fn sink(dialect: Dialect) -> rerail_tape_format::Named {
    rerail_tape_format::Named::new(tape_tag(dialect), TERMINATORS, names)
}

/// `sink`'s tape: its tree is the language's (a code its table lacks is the parser's bug).
pub(crate) fn written(t: Result<Tape, String>) -> Tape {
    t.unwrap_or_else(|e| panic!("the parser's tree is no format 16 tape of its language's table: {e}"))
}

/// The language read: C (tree-sitter-c) or C++ (tree-sitter-cpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    C,
    Cpp,
}

/// The supertype sets a node is in, as the tape names them (in the tape's order).
pub const CLASSES: &[&[&str]] = &[
    &[],
    &["expression"],
    &["statement"],
    &["type_specifier"],
    &["_declarator"],
    &["_field_declarator"],
    &["_type_declarator"],
    &["_abstract_declarator"],
];

/// The supertype-set value that marks an extra (a comment: no field, no supertypes). The writer's
/// keys are 19 bits: kind 9, field 6, set 4.
pub const EXTRA_CLASS: u8 = 15;

/// A node's key in the writer: its kind, field, supertype set (`CLASSES`), and the extra flag (set
/// `EXTRA_CLASS`).
#[inline(always)]
pub const fn key(kind: K, field: u8, class: u8, extra: bool) -> u32 {
    let class = if extra { EXTRA_CLASS } else { class };
    kind as u32 | (field as u32) << 9 | (class as u32) << 15
}

/// A key's kind, field, supertype set and extra flag (`key`'s inverse).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    let class = ((key >> 15) & 0xf) as u8;
    let extra = class == EXTRA_CLASS;
    ((key & 0x1ff) as u16, ((key >> 9) & 0x3f) as u8, if extra { 0 } else { class }, extra)
}

/// The tape of `src`: tree-sitter-c's or tree-sitter-cpp's, as the engine encodes it. `Err` where
/// the parser leaves the file to tree-sitter (a syntax error, a construct it does not read exactly).
pub fn tape(src: &[u8], dialect: Dialect) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src, dialect)
}

/// [`tape`], the parser's writer writing to `finish`'s target and finished by it: format 16 or 16z, written by its ops
/// as the parse goes (`Writer::finish_16`, decision 0434).
pub fn tape_into<T>(src: &[u8], dialect: Dialect, finish: &mut dyn rerail_tape_format::Sink<T>) -> Result<T, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_into(src, dialect, finish)
}

/// The tape of `src` read from `toks` (ending in `Eof`), which are `src`'s tokens as the lexer
/// would make them: a preprocessor's tokens over its expanded text (decision 0327), parsed with no
/// lexing of the text.
pub fn tape_of_tokens(src: &[u8], toks: &[Tok], dialect: Dialect) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_tokens(src, toks, dialect)
}

/// Stage 1 alone over `src` (its masks), for the cost examples: the text's first NUL, if any.
#[doc(hidden)]
pub fn stage1_only(src: &[u8]) -> Option<usize> {
    block::Blocks::of(src).first_nul()
}

/// The tokens of `src`, lexed alone (no parse), as the parser's buffer lexes them: their count,
/// and comments; `None` where the lexer refuses one. For the cost examples (`*_costs lex`).
#[doc(hidden)]
pub fn lex_only(src: &[u8], dialect: Dialect) -> Option<(usize, usize)> {
    let lx = lex::Lexer::new(src, dialect == Dialect::Cpp);
    let (mut at, mut n, mut extras, mut toks) = (0, 0, Vec::new(), Vec::with_capacity(16));
    loop {
        toks.clear();
        at = lx.run(at, &mut extras, &mut toks, 16);
        n += toks.len();
        match toks.last() {
            Some(t) if t.kind == lex::T::Eof && t.word == u8::MAX => return None,
            Some(t) if t.kind == lex::T::Eof => return Some((n, extras.len())),
            _ => {}
        }
    }
}

/// A tape as plain text, as the engine's `python_tape_plain` writes one (the oracle's form): a node
/// a line, indented by depth, its kind, field, flags, supertypes and span.
pub fn tape_text(tape: &Tape) -> String {
    tape.tape.text().unwrap_or_else(|e| panic!("a tape this crate wrote: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_fit_the_keys() {
        assert!(KINDS.len() < 512 && FIELDS.len() <= 64 && CLASSES.len() < EXTRA_CLASS as usize);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Identifier, f::NAME, 6, false)), (K::Identifier as u16, f::NAME, 6, false));
        assert_eq!(unkey(key(K::Comment, 0, 0, true)), (K::Comment as u16, 0, 0, true));
        assert!(key(kinds::LAST, f::VALUE, 9, true) < 1 << 19);
    }
}
