//! A lean HTML parser for the tape alone: tree-sitter-html's tape, as the engine encodes its trees
//! (named-only, decision 0140's writer), written as the parse goes. The same approach as the lean
//! Python, ECMAScript, Rust, Java, C# and PHP parsers: it stands in for tree-sitter where a text
//! parses; a text it cannot read exactly is left to tree-sitter (`Refusal`), which also recovers
//! from errors. There is one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`) gives the parser its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The parser** (`parse`) reads the text once, keeping tree-sitter-html's scanner's tag stack,
//!   and writes each node in preorder as it opens. A tag name's type is one binary search over the
//!   names the scanner knows (`tags`).
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS` and the extra flag (HTML's
//! grammar has no fields and no supertypes). The engine maps each to its grammar's ids by name.

mod block;
mod kinds;
mod parse;
pub mod tags;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS};
pub use rerail_tape_format::Columns as Tape;

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

/// The supertype sets a node is in: HTML's grammar has none.
pub const CLASSES: &[&[&str]] = &[&[]];

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

/// Whether the tape stores token `tok` under `parent` (`kinds::DROPPED`: the engine's automata).
#[inline]
pub(crate) fn stores(parent: K, tok: K) -> bool {
    kinds::DROPPED.binary_search(&(parent as u32 | (tok as u32) << 9)).is_err()
}

/// The tape of `src`: tree-sitter-html's, as the engine encodes it. `Err` where the parser leaves
/// the file to tree-sitter (a syntax error, a construct it does not read exactly).
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src)
}

/// [`tape`], the parser's writer finished by `finish`: format 15's columns (`tape`), or format 16 written
/// in one pass over it (`Writer::finish_16`, decision 0293).
pub fn tape_into<T>(src: &[u8], finish: &mut dyn FnMut(rerail_tape_format::Writer) -> Result<T, rerail_tape_format::Redo>) -> Result<T, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_into(src, finish)
}

/// A tape as plain text, as the engine's `python_tape_plain` writes one (the oracle's form): a node
/// a line, indented by depth, its kind, flags and span.
pub fn tape_text(tape: &Tape) -> String {
    use rerail_tape_format::TAIL_FROM_START;
    let cell = |column: &[u8], width: u8, i: usize| -> u32 {
        let w = usize::from(width & 0x7f);
        let b = &column[i * w..i * w + w];
        if w == 2 { u32::from(u16::from_le_bytes([b[0], b[1]])) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
    };
    let n = tape.code.len() / 2;
    let start = |i: usize| {
        let b = i / rerail_tape_format::CHECKPOINT_STRIDE;
        u32::from_le_bytes(tape.base[4 * b..4 * b + 4].try_into().expect("a base")) + cell(&tape.off, tape.widths[1], i)
    };
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..n {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let code = u16::from_le_bytes([tape.code[2 * i], tape.code[2 * i + 1]]);
        let (kind, _, _, extra) = unkey(tape.keys[usize::from(code)]);
        let size = cell(&tape.size, tape.widths[0], i) as usize;
        let s = start(i);
        let from = if tape.widths[2] & TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
        let end = from + cell(&tape.tail, tape.widths[2], i);
        out.push_str(&format!(
            "{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            KINDS[usize::from(kind)].0,
            if extra { " flags=1" } else { "" },
        ));
        open.push(i + size);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_fit_the_keys() {
        assert!(KINDS.len() < 512);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Comment, true)), (K::Comment as u16, 0, 0, true));
        assert!(kinds::DROPPED.windows(2).all(|w| w[0] < w[1]));
        // The parser writes `<` before it knows a start tag from a self-closing one.
        assert_eq!(stores(K::StartTag, K::TLt), stores(K::SelfClosingTag, K::TLt));
    }
}
