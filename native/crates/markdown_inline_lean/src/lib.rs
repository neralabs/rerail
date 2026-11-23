//! A lean parser of Markdown's inline grammar for the tape alone: tree-sitter-md-inline's tape, as
//! the engine encodes its trees (named-only, decision 0140's writer). The same approach as the other
//! lean parsers: it stands in for tree-sitter where a text parses; a text it cannot read exactly is
//! left to tree-sitter (`Refusal`). There is one behavior: tree-sitter's tape, or a refusal.
//!
//! The inline grammar is ambiguous by design: a `[` opens a link or is text, a `*` opens emphasis or
//! is a lone delimiter, a `<` opens an HTML tag or is text, and tree-sitter's GLR parser keeps each
//! reading as a version until it fails, picking the survivor of the highest dynamic precedence. The
//! parser (`parse`) does the same over its own small states: each version a stack of frames, the
//! versions advanced token by token in step, merged where they meet, scored by the grammar's dynamic
//! precedences. Where tree-sitter's choice is not the score's (a tie between different trees, more
//! versions than tree-sitter keeps), the text is refused.
//!
//! - **The scanner** (`scan`) is tree-sitter-md-inline's external scanner, ported: emphasis
//!   delimiters by their flanking, code and LaTeX spans by a closing run of the same length.
//! - **The lexer** (`parse::lex`) takes the grammar's own tokens where the scanner finds none: the
//!   longest match among the tokens the version's state takes.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS` (the inline grammar has no fields,
//! supertypes or extras). The engine maps each to its grammar's ids by name.

mod block;
mod kinds;
mod parse;
mod scan;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS};
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0626).
pub type Tape = rerail_tape_format::f16::Finished;

/// The language's tape tag.
pub const TAPE_TAG: u8 = 23;

/// The line terminators of the language's ecosystem (decision 0612).
pub const TERMINATORS: rerail_tape_format::newlines::Terminators = rerail_tape_format::newlines::Terminators::Newlines;

/// The crate's own sink (decision 0626): format 16, each key numbered in the language's table by its names, as the
/// engine numbers it by the grammar's ids (`rerail_engine`'s `formats::lean`), so the tapes are the same.
pub fn sink() -> rerail_tape_format::Named {
    rerail_tape_format::Named::new(TAPE_TAG, TERMINATORS, |key| {
        let (kind, field, class, extra) = unkey(key);
        let (kind, named) = *KINDS.get(usize::from(kind))?;
        Some(rerail_tape_format::KeyNames {
            kind,
            named,
            field: FIELDS.get(usize::from(field)).copied()?,
            flags: if extra { rerail_tape_format::CODE_EXTRA } else { 0 },
            classes: if extra { &[] } else { CLASSES.get(usize::from(class)).copied()? },
        })
    })
}

/// `sink`'s tape: its tree is the language's (a code its table lacks is the parser's bug).
pub(crate) fn written(t: Result<Tape, String>) -> Tape {
    t.unwrap_or_else(|e| panic!("the parser's tree is no format 16 tape of its language's table: {e}"))
}

/// Why a text is left to tree-sitter: where the parser stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A byte the parser does not read (a NUL, a text that is not UTF-8).
    Lex(u32),
    /// A construct not read exactly, an ambiguity tree-sitter settles otherwise, or a syntax error.
    Parse(u32),
    /// A text too large for the tape's widths.
    TooLarge,
}

/// The supertype sets a node is in: the inline grammar has none.
pub const CLASSES: &[&[&str]] = &[&[]];

/// The supertype-set value that marks an extra (none here). The writer's keys are 19 bits: kind 9,
/// field 6, set 4.
pub const EXTRA_CLASS: u8 = 15;

/// A node's key in the writer: its kind (the grammar has no fields).
#[inline(always)]
pub const fn key(kind: K) -> u32 {
    kind as u32
}

/// A key's kind, field, supertype set and extra flag (`key`'s inverse, in the other lean parsers'
/// form).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    let class = ((key >> 15) & 0xf) as u8;
    ((key & 0x1ff) as u16, ((key >> 9) & 0x3f) as u8, 0, class == EXTRA_CLASS)
}

/// The tape of `src`, an inline text (an `inline` or `pipe_table_cell` node's text, as the engine
/// cuts it): tree-sitter-md-inline's, as the engine encodes it. `Err` where the parser leaves the
/// text to tree-sitter.
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src)
}

/// [`tape`], the parser's writer writing to `finish`'s target and finished by it: format 15's columns (`tape`), or
/// format 16 written by its ops as the parse goes (`Writer::finish_16`, decision 0434).
pub fn tape_into<T>(src: &[u8], finish: &mut dyn rerail_tape_format::Sink<T>) -> Result<T, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_into(src, finish)
}

/// A tape as plain text, as the engine's `python_tape_plain` writes one (the oracle's form): a node
/// a line, indented by depth, its kind and span.
pub fn tape_text(tape: &Tape) -> String {
    tape.tape.text().unwrap_or_else(|e| panic!("a tape this crate wrote: {e}"))
}
