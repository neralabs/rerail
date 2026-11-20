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

/// [`tape`], the parser's writer writing to `finish`'s target and finished by it: format 15's columns (`tape`), or
/// format 16 written by its ops as the parse goes (`Writer::finish_16`, decision 0426).
pub fn tape_into<T>(src: &[u8], finish: &mut dyn rerail_tape_format::Sink<T>) -> Result<T, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    parse::parse_into(src, finish)
}

/// The node kinds a parse may start at (decision 0482): a whole document, or one element alone ([`tape_from_into`]).
pub const STARTS: &[&str] = &["document", "element"];

/// [`tape_into`] from the node kind `start` (one of [`STARTS`]): its tape's root is one node of that kind over `src`,
/// less blanks. `None` for a kind it does not start at.
pub fn tape_from_into<T>(src: &[u8], start: &str, finish: &mut dyn rerail_tape_format::Sink<T>) -> Option<Result<T, Refusal>> {
    let start = match start {
        "document" => parse::Start::Document,
        "element" => parse::Start::Element,
        _ => return None,
    };
    if src.len() >= u32::MAX as usize / 2 {
        return Some(Err(Refusal::TooLarge));
    }
    Some(parse::parse_from(src, start, finish))
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

    fn element(src: &str) -> Result<Tape, Refusal> {
        tape_from_into(src.as_bytes(), "element", &mut rerail_tape_format::Writer::<rerail_tape_format::Widths>::finish_columns).unwrap()
    }

    /// Decision 0482 R1: an element alone is one `element` root over the text less blanks, nothing above it, and the
    /// same nodes as the document's element of the same text (no `html`, `head` or `body` implied around it).
    #[test]
    fn an_element_fragment_is_rooted_at_its_element() {
        for (src, span) in [
            ("<div class=\"a\">x &amp; <b>y</b></div>", "[0..37]"),
            ("  <ul><li>a<li>b</ul>\n", "[2..21]"),
            ("<br>", "[0..4]"),
            ("<td>x</td>", "[0..10]"),
        ] {
            let ours = tape_text(&element(src).unwrap_or_else(|e| panic!("{src:?}: {e:?}")));
            let first = ours.lines().next().unwrap();
            assert!(first.starts_with("element") && first.ends_with(span), "{src:?}:\n{ours}");
            assert!(ours.lines().skip(1).all(|l| l.starts_with(' ')), "one root: {src:?}:\n{ours}");
            // Where the document's element ends where the fragment's does, its nodes are the fragment's.
            let doc = tape_text(&tape(src.trim_end().as_bytes()).unwrap());
            let theirs: Vec<String> = doc.lines().skip(1).map(|l| l[2..].to_owned()).collect();
            assert_eq!(ours.lines().collect::<Vec<_>>(), theirs.iter().map(String::as_str).collect::<Vec<_>>(), "{src:?}");
        }
    }

    /// Refused: no element first, another node after it, an end tag that closes nothing, a `script` or `style` (kinds of their own), an element the scanner does not
    /// end at the text's end (as in a document).
    #[test]
    fn an_element_fragment_is_one_element() {
        for src in ["", "x", "<!-- c --><a></a>", "<!doctype html>", "<a></a><b></b>", "<a></a> x", "<p>a<div>b</div>", "</a>", "<div>", "<p>a ", "<script>if (a < b) {}</script>", "<style>a {}</style>"] {
            assert!(element(src).is_err(), "{src:?}");
        }
        assert!(tape_from_into(b"<a></a>", "text", &mut rerail_tape_format::Writer::<rerail_tape_format::Widths>::finish_columns).is_none());
    }
}
