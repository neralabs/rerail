#![recursion_limit = "1024"]
//! A lean Swift parser for the tape alone: tree-sitter-swift's tape, as the engine encodes its
//! trees (named-only, decision 0140's writer), written as the parse goes; the lean Kotlin parser's
//! shape (decision 0305). A file it cannot read exactly is left to tree-sitter (`Refusal`). There is
//! one behavior: tree-sitter's tape, or a refusal.
//!
//! - **Stage 1** (`block`, shared with `rerail-kotlin-lean`) gives the lexer its byte classes 64
//!   bytes a step (`rerail-simd8`).
//! - **The lexer** (`lex`) reads a token where the parser asks. tree-sitter-swift has no keyword
//!   extraction: a word is a keyword where the parse state takes it, so the parser decides. Its
//!   external scanner is ported: operators as runs of operator characters, the implicit semicolon
//!   (`Lexer::semi`), nested block comments, raw strings.
//! - **The parser** (`parse`) is recursive descent, its expressions read by the precedences and
//!   associativities tree-sitter's parse table resolves (a call binds looser than `+`; the right
//!   operand of a comparison takes every operator after it), writing each node in preorder through
//!   the writer the three lean parsers share (`tw`).

#[path = "../../kotlin_lean/src/block.rs"]
mod block;
mod kinds;
mod lex;
mod parse;
#[path = "../../kotlin_lean/src/tw.rs"]
mod tw;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS, f};
pub use lex::Refusal;
pub use rerail_tape_format::Columns as Tape;
pub use tw::key;

/// A key's kind, field, supertype set and extra flag (`key`'s inverse, in the other lean parsers'
/// form; tree-sitter-swift has no supertypes).
pub const fn unkey(key: u32) -> (u16, u8, u8, bool) {
    ((key & 0x3ff) as u16, ((key >> 10) & 0x3f) as u8, 0, key >> 16 & 1 == 1)
}

/// The supertype sets a node is in, as the tape names them: tree-sitter-swift has none.
pub const CLASSES: &[&[&str]] = &[&[]];

/// The tape of `src`: tree-sitter-swift's, as the engine encodes it. `Err` where the parser
/// leaves the file to tree-sitter (a syntax error, a construct it does not read exactly).
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
        assert!(KINDS.len() < 1024 && FIELDS.len() <= 64);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Identifier, f::NAME, false)), (K::Identifier as u16, f::NAME, 0, false));
        assert_eq!(unkey(key(K::Comment, 0, true)), (K::Comment as u16, 0, 0, true));
    }
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
