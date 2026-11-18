//! A lean Markdown parser for the tape alone: tree-sitter-md's block tape, as the engine encodes its
//! trees (named-only, decision 0140's writer), written as the parse goes. The same approach as the
//! lean Python, ECMAScript, Rust, Java, C#, PHP, HTML and CSS parsers: it stands in for tree-sitter
//! where a text parses; a text it cannot read exactly is left to tree-sitter (`Refusal`), which also
//! recovers from errors. There is one behavior: tree-sitter's tape, or a refusal.
//!
//! - **The scanner** (`scan`) is tree-sitter-md's external scanner, ported line by line: the block
//!   structure (the stack of open blocks, matched at each line's start) is decided there.
//! - **The parser** (`parse`) follows the block grammar, asking the scanner first at each token
//!   with the external tokens its state takes, and lexing the grammar's own tokens (words, white
//!   space, punctuation) where the scanner finds none.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS` and a field from `FIELDS`
//! (Markdown's grammar has no supertypes and no extras). The engine maps each to its grammar's ids
//! by name.

mod block;
mod kinds;
mod parse;
mod scan;

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

/// The supertype sets a node is in: Markdown's grammar has none.
pub const CLASSES: &[&[&str]] = &[&[]];

/// The supertype-set value that marks an extra (none in Markdown). The writer's keys are 19 bits:
/// kind 9, field 6, set 4.
pub const EXTRA_CLASS: u8 = 15;

/// A node's key in the writer: its kind and field.
#[inline(always)]
pub const fn key(kind: K, field: u8) -> u32 {
    kind as u32 | (field as u32) << 9
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

/// The tape of `src`: tree-sitter-md's, as the engine encodes it. `Err` where the parser leaves
/// the file to tree-sitter (a syntax error, a construct it does not read exactly).
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse(src)
}

/// [`tape`], the parser's writer writing to `finish`'s target and finished by it: format 15's columns (`tape`), or
/// format 16 written by its ops as the parse goes (`Writer::finish_16`, decision 0399).
pub fn tape_into<T>(src: &[u8], finish: &mut dyn rerail_tape_format::Sink<T>) -> Result<T, Refusal> {
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
        let (kind, field, _, extra) = unkey(tape.keys[usize::from(code)]);
        let size = cell(&tape.size, tape.widths[0], i) as usize;
        let s = start(i);
        let from = if tape.widths[2] & TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
        let end = from + cell(&tape.tail, tape.widths[2], i);
        out.push_str(&format!(
            "{}{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            KINDS[usize::from(kind)].0,
            if field == 0 { String::new() } else { format!(" {}:", FIELDS[usize::from(field)]) },
            if extra { " flags=1" } else { "" },
        ));
        open.push(i + size);
    }
    out
}

/// Each node of a tape in preorder: its kind, span and depth (`tape_text`'s reading).
#[cfg(test)]
fn tape_nodes(tape: &Tape) -> Vec<(u16, u32, u32, usize)> {
    tape_text(tape)
        .lines()
        .map(|l| {
            let depth = (l.len() - l.trim_start().len()) / 2;
            let name = l.trim_start().split([' ']).next().expect("a kind");
            let kind = KINDS.iter().position(|k| k.0 == name).expect("a kind's name") as u16;
            let span = &l[l.rfind('[').expect("a span") + 1..l.len() - 1];
            let (s, e) = span.split_once("..").expect("s..e");
            (kind, s.parse().expect("start"), e.parse().expect("end"), depth)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inline guests of the files in `RERAIL_MD_FILES` (a list of paths), a file each in
    /// `RERAIL_MD_INLINE_OUT`, and their list (`list.txt`): the input of the inline grammar's
    /// oracle. A guest's text is the engine's (`guests::markdown_inline_text`): an `inline` or
    /// `pipe_table_cell` node's bytes, less its `block_continuation` children after its first child.
    #[test]
    #[ignore = "writes the inline guests' texts; needs RERAIL_MD_FILES and RERAIL_MD_INLINE_OUT"]
    fn inline_texts() {
        let list = std::fs::read_to_string(std::env::var("RERAIL_MD_FILES").expect("set RERAIL_MD_FILES")).unwrap();
        let out = std::path::PathBuf::from(std::env::var("RERAIL_MD_INLINE_OUT").expect("set RERAIL_MD_INLINE_OUT"));
        std::fs::create_dir_all(&out).unwrap();
        let (inline, cell, cont) = (K::Inline as u16, K::PipeTableCell as u16, K::BlockContinuation as u16);
        let mut paths = String::new();
        let (mut files, mut texts) = (0, 0);
        for (n, path) in list.lines().filter(|l| !l.is_empty()).enumerate() {
            let Ok(src) = std::fs::read(path) else { continue };
            let Ok(t) = tape(&src) else { continue };
            files += 1;
            let nodes = tape_nodes(&t);
            let mut k = 0;
            for (i, &(kind, s, e, depth)) in nodes.iter().enumerate() {
                if kind != inline && kind != cell {
                    continue;
                }
                let kids = nodes[i + 1..].iter().take_while(|x| x.3 > depth).filter(|x| x.3 == depth + 1);
                let (mut text, mut from) = (Vec::new(), s as usize);
                for &(ck, cs, ce, _) in kids.skip(1) {
                    if ck == cont {
                        text.extend_from_slice(&src[from..cs as usize]);
                        from = ce as usize;
                    }
                }
                text.extend_from_slice(&src[from..e as usize]);
                let file = out.join(format!("{n:05}-{k:04}.md"));
                std::fs::write(&file, &text).unwrap();
                paths.push_str(&format!("{}\n", file.display()));
                k += 1;
                texts += 1;
            }
        }
        std::fs::write(out.join("list.txt"), paths).unwrap();
        eprintln!("{files} files, {texts} inline texts");
    }

    #[test]
    fn tables_fit_the_keys() {
        assert!(KINDS.len() < 512);
        assert_eq!(KINDS.len(), kinds::LAST as usize + 1);
        assert_eq!(unkey(key(K::Inline, kinds::f::HEADING_CONTENT)), (K::Inline as u16, kinds::f::HEADING_CONTENT, 0, false));
    }

    #[test]
    fn a_heading_and_a_paragraph() {
        let t = tape(b"# Title\n\nSome *text*.\n").unwrap();
        assert_eq!(
            tape_text(&t),
            "document [0..22]\n  section [0..22]\n    atx_heading [0..8]\n      atx_h1_marker [0..1]\n      inline heading_content: [2..7]\n    paragraph [9..22]\n      inline [9..21]\n        * [14..15]\n        * [19..20]\n        . [20..21]\n"
        );
    }
}
