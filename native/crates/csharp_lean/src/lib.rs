//! A lean C# parser for the tape alone: tree-sitter-c-sharp's tape, as the engine encodes its trees
//! (named-only, decision 0140's writer), written as the parse goes. The same approach as
//! `rerail-python-lean`, `rerail-ecma-lean`, `rerail-rust-lean` and `rerail-java-lean`: it stands in for tree-sitter
//! where a file parses; a file it cannot read exactly is left to tree-sitter (`Refusal`), which also
//! recovers from errors. There is one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`) gives the lexer its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The lexer** (`lex`) reads a token where the parser asks; every word is a name with a word
//!   code from one hash probe, and the parser decides where it is a keyword (tree-sitter's keyword
//!   extraction). A string of any form is one token; the parser reads its pieces from its bytes, as
//!   tree-sitter-c-sharp's external scanner does.
//! - **The parser** (`parse`) is recursive descent, writing each node in preorder as it opens, with
//!   its code (kind, field, supertypes); a node found after its first child (an operator after its
//!   left operand) is put before it (`Writer::insert`). Only the tokens tree-sitter's tape stores
//!   are written: the engine's implied-token automata say which (`kinds::DROPPED`, and a gap's
//!   delimiters by its shape).
//! - **Extras** (comments and the preprocessor's lines but `#if`) are written when the parse next
//!   opens a node or reads a token, at the node then open. `#if` … `#endif` are the grammar's
//!   (`preproc_if`), read where the grammar takes them.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS`, a set of
//! supertypes from `CLASSES`, and the extra flag. The engine maps each to its grammar's ids by name.

// The word table's macro recurses once a word (`lex::w`).
#![recursion_limit = "512"]

mod block;
mod kinds;
mod lex;
mod parse;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS, f};
pub use lex::Refusal;
pub use rerail_tape_format::Columns as Tape;

/// The supertype sets a node is in, as the tape names them (in the tape's order).
pub const CLASSES: &[&[&str]] = &[
    &[],
    &["expression", "lvalue_expression"],
    &["expression", "non_lvalue_expression"],
    &["expression", "non_lvalue_expression", "literal"],
    &["lvalue_expression"],
    &["non_lvalue_expression"],
    &["non_lvalue_expression", "literal"],
    &["literal"],
    &["type"],
    &["statement"],
    &["declaration"],
    &["pattern"],
    &["type_declaration"],
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

/// The tape of `src`: tree-sitter-c-sharp's, as the engine encodes it. `Err` where the parser leaves
/// the file to tree-sitter (a syntax error, a construct it does not read exactly).
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src)
}

/// A tape as plain text, as the engine's `python_tape_plain` writes one (the oracle's form): a node
/// a line, indented by depth, its kind, field, flags, supertypes and span.
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
        let (kind, field, class, extra) = unkey(tape.keys[usize::from(code)]);
        let size = cell(&tape.size, tape.widths[0], i) as usize;
        let s = start(i);
        let from = if tape.widths[2] & TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
        let end = from + cell(&tape.tail, tape.widths[2], i);
        let classes = CLASSES[usize::from(class)];
        out.push_str(&format!(
            "{}{}{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            KINDS[usize::from(kind)].0,
            if field == 0 { String::new() } else { format!(" {}:", FIELDS[usize::from(field)]) },
            if extra { " flags=1".to_owned() } else { String::new() },
            if classes.is_empty() { String::new() } else { format!(" {{{}}}", classes.join(",")) },
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
        assert!(KINDS.len() < 512 && FIELDS.len() <= 64 && CLASSES.len() < EXTRA_CLASS as usize);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Identifier, f::NAME, 6, false)), (K::Identifier as u16, f::NAME, 6, false));
        assert_eq!(unkey(key(K::Comment, 0, 0, true)), (K::Comment as u16, 0, 0, true));
        assert!(key(kinds::LAST, f::VALUE, 9, true) < 1 << 19);
    }
}
