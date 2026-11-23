//! A lean SQL parser for the tape alone: tree-sitter-sql's tape, as the engine encodes its trees
//! (named-only, decision 0140's writer). The same approach as the lean Python, ECMAScript, Rust, Java,
//! C#, PHP, HTML, CSS and Markdown parsers: it stands in for tree-sitter where a text parses; a text it
//! cannot read exactly is left to tree-sitter (`Refusal`), which also recovers from errors. There is
//! one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`) gives the lexer its byte classes 64 bytes a step (`rerail-simd8`): runs
//!   of white space and of word bytes, a line's end, a string's stops.
//! - **The parser** (`parse`) is recursive descent over tree-sitter-sql's grammar. A word is a
//!   keyword where the grammar takes that keyword at that point, as tree-sitter's keyword lexing
//!   does (its `word` token), else an identifier; where the parser cannot tell, it refuses.
//! - **The tape** is written as the text is read, each node in preorder (`rerail_tape_format::Writer`,
//!   decision 0226), with no tree built first. Each node's tokens are stored as the engine's token
//!   automata store them (`kinds::MOVES`, from the engine's `TokenGrammar`), and each comment goes
//!   in the node open when the next token or node is written, as tree-sitter's reductions leave it.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS` and the extra
//! flag. The engine maps each to its grammar's ids by name.

mod block;
#[allow(dead_code)] // the generated tables hold every kind and field
mod kinds;
mod parse;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS};
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0631).
pub type Tape = rerail_tape_format::f16::Finished;

/// The language's tape tag.
pub const TAPE_TAG: u8 = 10;

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

/// Why a text is left to tree-sitter: where the parser stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A byte the parser does not read (a NUL).
    Lex(u32),
    /// A construct not read exactly, or a syntax error.
    Parse(u32),
    /// A text too large for the tape's widths.
    TooLarge,
}

/// The supertype sets a node is in: SQL's grammar has none.
pub const CLASSES: &[&[&str]] = &[&[]];

/// A node's key in the writer: its kind (10 bits), its field (6) and whether it is an extra (a
/// comment, a psql meta-command).
#[inline(always)]
pub const fn key(kind: K, field: u8, extra: bool) -> u32 {
    kind as u32 | (field as u32) << 10 | (extra as u32) << 16
}

/// A key's kind, field, supertype set and extra flag (`key`'s inverse, in the other lean parsers'
/// form).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    ((key & 0x3ff) as u16, ((key >> 10) & 0x3f) as u8, 0, key >> 16 & 1 == 1)
}

/// The tape of `src`: tree-sitter-sql's, as the engine encodes it. `Err` where the parser leaves
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
        assert!(KINDS.len() < 1024);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert!(FIELDS.len() < 64);
        assert_eq!(unkey(key(K::Identifier, kinds::f::NAME, true)), (K::Identifier as u16, kinds::f::NAME, 0, true));
    }

    #[test]
    fn a_select() {
        let t = tape(b"SELECT a FROM t;\n").unwrap();
        assert_eq!(
            tape_text(&t),
            "program [0..17]\n  statement [0..15]\n    select [0..8]\n      keyword_select [0..6]\n      select_expression [7..8]\n        term [7..8]\n          field value: [7..8]\n            identifier name: [7..8]\n    from [9..15]\n      keyword_from [9..13]\n      relation [14..15]\n        object_reference [14..15]\n          identifier name: [14..15]\n  ; [15..16]\n"
        );
    }

    /// A profiling loop: the lean parser over `RERAIL_SQL_FILES` (a list of paths), the files it reads,
    /// `RERAIL_SQL_PASSES` times (default 2,000), for a sampling profiler to watch.
    #[test]
    #[ignore = "a profiling loop; needs RERAIL_SQL_FILES"]
    fn profile_loop() {
        let list = std::fs::read_to_string(std::env::var("RERAIL_SQL_FILES").expect("set RERAIL_SQL_FILES")).unwrap();
        let passes: usize = std::env::var("RERAIL_SQL_PASSES").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
        let files: Vec<Vec<u8>> = list.lines().filter_map(|p| std::fs::read(p).ok()).filter(|f| crate::tape(f).is_ok()).collect();
        let mut n = 0;
        for _ in 0..passes {
            for f in &files {
                n += crate::tape(f).map_or(0, |t| t.tape.len());
            }
        }
        eprintln!("{} files, {n} nodes", files.len());
    }
}
