//! A lean Vue single-file component parser for the tape alone: tree-sitter-vue's tape, as the
//! engine encodes its trees (named-only, decision 0140's writer), written as the parse goes. The
//! same approach as the other lean parsers: it stands in for tree-sitter where a file parses; a
//! file it cannot read exactly is left to tree-sitter (`Refusal`), which also recovers from errors.
//! There is one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`) gives the parser its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The parser** (`parse`) reads the file once, keeping tree-sitter-vue's scanner's tag stack
//!   (the tag rules are tree-sitter-html's: `rerail_html_lean::tags`), and writes each node in
//!   preorder as it opens. Which tokens the tape keeps, the engine's token automata decide
//!   (`kinds`, from `automata.tsv`), a gap at a time.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS` and the extra flag (the grammar
//! has no fields and no supertypes). The engine maps each to its grammar's ids by name.

mod block;
mod kinds;
mod parse;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS};
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0631).
pub type Tape = rerail_tape_format::f16::Finished;

/// The language's tape tag.
pub const TAPE_TAG: u8 = 12;

/// The line terminators of the language's ecosystem (decision 0612).
pub const TERMINATORS: rerail_tape_format::newlines::Terminators = rerail_tape_format::newlines::Terminators::Newlines;

/// Key `key`'s names, as the grammar names them: what its number in its language's table is found by (decision
/// 0631; `sink`, and the engine's `formats::lean_named`). `None` for a key the parser does not write.
pub fn names(key: u32) -> Option<rerail_tape_format::KeyNames> {
    let (kind, field, _, extra) = unkey(key);
    let (kind, named) = *KINDS.get(usize::from(kind))?;
    Some(rerail_tape_format::KeyNames {
        kind,
        named,
        field: FIELDS.get(usize::from(field)).copied()?,
        flags: if extra { rerail_tape_format::CODE_EXTRA } else { 0 },
        classes: &[],
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

/// Why a file is left to tree-sitter: where the parser stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A byte the parser does not read (a NUL, a white space past ASCII where it ends a token).
    Lex(u32),
    /// A construct not read exactly, or a syntax error.
    Parse(u32),
    /// A file too large for the tape's widths.
    TooLarge,
}

/// The supertype-set value that marks an extra (a comment). The writer's keys are 19 bits: kind 9,
/// field 6, set 4.
pub const EXTRA_CLASS: u8 = 15;

/// A node's key in the writer: its kind, and the extra flag.
#[inline(always)]
pub const fn key(kind: K, extra: bool) -> u32 {
    kind as u32 | if extra { (EXTRA_CLASS as u32) << 15 } else { 0 }
}

/// A key's kind, field, supertype set and extra flag (`key`'s inverse, in the other lean parsers'
/// form).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    let class = ((key >> 15) & 0xf) as u8;
    ((key & 0x1ff) as u16, ((key >> 9) & 0x3f) as u8, 0, class == EXTRA_CLASS)
}

/// The tape of `src`: tree-sitter-vue's, as the engine encodes it. `Err` where the parser leaves
/// the file to tree-sitter (a syntax error, a construct it does not read exactly).
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
/// a line, indented by depth, its kind, flags and span.
pub fn tape_text(tape: &Tape) -> String {
    tape.tape.text().unwrap_or_else(|e| panic!("a tape this crate wrote: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_fit_the_keys() {
        assert!(KINDS.len() < 512);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Comment, true)), (K::Comment as u16, 0, 0, true));
    }
}
