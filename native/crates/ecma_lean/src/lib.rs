//! A lean JavaScript, TypeScript and TSX parser for the tape alone: tree-sitter-javascript's,
//! -typescript's and -tsx's tape, as the engine encodes their trees (named-only, decision 0140's
//! writer), written as the parse goes. The same approach as `rerail-python-lean`: it stands in for
//! tree-sitter where a file parses; a file it cannot read exactly is left to tree-sitter
//! (`Refusal`), which also recovers from errors. There is one behavior: tree-sitter's tape, or a
//! refusal.
//!
//! - **Stage 1** (`block`) gives the lexer its byte classes 64 bytes a step (`rerail-simd8`).
//! - **The lexer** (`lex`) reads a token where the parser asks, in the mode its state reads (a `/`
//!   is a regular expression only where an operand may start); every word is a name with a word
//!   code from one hash probe, and the parser decides where it is a keyword.
//! - **The parser** (`parse`) is recursive descent, writing each node in preorder as it opens, with
//!   its code (kind, field, supertypes); a node found after its first child (an operator after its
//!   left operand) is put before it (`Writer::insert`). Only the tokens tree-sitter's tape stores
//!   are written. Automatic semicolons are tree-sitter's external scanner's, ported (`parse::asi`).
//! - **Extras** (comments) are written when the parse next opens a node or reads a token, at the
//!   node then open.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS`, a set of
//! supertypes from `CLASSES`, and the extra flag. The engine maps each to its grammar's ids by name.

mod block;
mod kinds;
mod lex;
mod parse;

#[cfg(test)]
mod oracle;

pub use kinds::{FIELDS, K, KINDS, f};
pub use lex::Refusal;
/// A tape this crate writes (`tape`): format 16 in its language's table, numbered by names (`sink`; decision 0631).
pub type Tape = rerail_tape_format::f16::Finished;

/// Each dialect's tape tag.
pub const fn tape_tag(dialect: Dialect) -> u8 {
    match dialect {
        Dialect::JavaScript => 6, Dialect::TypeScript => 2, Dialect::Tsx => 3,
    }
}

/// The line terminators of the language's ecosystem (decision 0612).
pub const TERMINATORS: rerail_tape_format::newlines::Terminators = rerail_tape_format::newlines::Terminators::Eslint;

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

/// Which grammar the text is read by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// tree-sitter-javascript (JSX included).
    JavaScript,
    /// tree-sitter-typescript's `typescript` (no JSX; `<T>x` is a type assertion).
    TypeScript,
    /// tree-sitter-typescript's `tsx` (JSX; no type assertions).
    Tsx,
}

/// The supertype sets a node is in, as the tape names them (in the tape's order).
pub const CLASSES: &[&[&str]] = &[
    &[],
    &["expression", "primary_expression"],
    &["expression"],
    &["primary_expression"],
    &["declaration", "statement"],
    &["statement"],
    &["pattern"],
    &["declaration"],
    &["type", "primary_type"],
    &["type"],
    &["primary_type"],
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

/// The tape of `src` in `dialect`: tree-sitter's, as the engine encodes it. `Err` where the parser
/// leaves the file to tree-sitter (a syntax error, a construct it does not read exactly).
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

/// The node kinds a parse may start at (decision 0466): a whole file, or one JSX element alone ([`tape_from_into`]).
pub const STARTS: &[&str] = &["program", "jsx_element", "jsx_self_closing_element"];

/// [`tape_into`] from the node kind `start` (one of [`STARTS`]): its tape's root is one node of that kind over `src`,
/// less blanks. `None` for a kind it does not start at.
pub fn tape_from_into<T>(src: &[u8], dialect: Dialect, start: &str, finish: &mut dyn rerail_tape_format::Sink<T>) -> Option<Result<T, Refusal>> {
    let mut kept = rerail_tape_format::Kept::new(finish);
    tape_from_into_kept(src, dialect, start, &mut kept).map(|r| r.map(|()| kept.take()))
}

/// `tape_from_into` writing through `Sink<()>` (`rerail_tape_format::Kept`): the parse is one instance, compiled in this
/// crate (optimized in dev builds), whatever the caller's output.
#[inline(never)]
fn tape_from_into_kept(src: &[u8], dialect: Dialect, start: &str, finish: &mut dyn rerail_tape_format::Sink<()>) -> Option<Result<(), Refusal>> {
    let start = match start {
        "program" => parse::Start::Program,
        "jsx_element" => parse::Start::JsxElement,
        "jsx_self_closing_element" => parse::Start::JsxSelfClosingElement,
        _ => return None,
    };
    if src.len() >= u32::MAX as usize / 2 {
        return Some(Err(Refusal::TooLarge));
    }
    Some(parse::parse_from(src, dialect, start, finish))
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

    fn element(src: &str, start: &str) -> Result<Tape, Refusal> {
        tape_from_into(src.as_bytes(), Dialect::JavaScript, start, &mut sink(Dialect::JavaScript)).unwrap().map(written)
    }

    /// Decision 0466 R1: a JSX element alone is one root of its kind over the text less blanks, nothing above it.
    #[test]
    fn a_jsx_fragment_is_rooted_at_its_element() {
        for (src, start, root) in [
            ("<a href={x}>hi {y}</a>", "jsx_element", "jsx_element [0..22]"),
            ("  <>a<b/></>\n", "jsx_element", "jsx_element [2..12]"),
            ("<br />", "jsx_self_closing_element", "jsx_self_closing_element [0..6]"),
        ] {
            let text = tape_text(&element(src, start).unwrap_or_else(|e| panic!("{src:?}: {e:?}")));
            assert!(text.lines().next().unwrap().starts_with(root.split(' ').next().unwrap()), "{src:?}:\n{text}");
            assert!(text.lines().next().unwrap().ends_with(root.split(' ').nth(1).unwrap()), "{src:?}:\n{text}");
            assert!(text.lines().skip(1).all(|l| l.starts_with(' ')), "one root: {src:?}:\n{text}");
        }
    }

    /// The element is the expression's: the same nodes as in `x = <…>;` at the element.
    #[test]
    fn a_jsx_fragment_is_the_files_element() {
        let src = "<a b=\"c\">{d}<e /></a>";
        let ours = tape_text(&element(src, "jsx_element").unwrap());
        let file = tape_text(&tape(format!("x = {src};").as_bytes(), Dialect::JavaScript).unwrap());
        let lines: Vec<&str> = file.lines().collect();
        let at = lines.iter().position(|l| l.trim_start().starts_with("jsx_element")).unwrap();
        let depth = lines[at].len() - lines[at].trim_start().len();
        let shift = |l: &str| {
            let (head, span) = l.rsplit_once(" [").unwrap();
            let (a, b) = span.trim_end_matches(']').split_once("..").unwrap();
            let head = &head[depth..];
            let head = head.split_once(" right:").map_or(head.to_owned(), |(k, rest)| format!("{k}{rest}"));
            format!("{head} [{}..{}]", a.parse::<u32>().unwrap() - 4, b.parse::<u32>().unwrap() - 4)
        };
        let theirs: Vec<String> = std::iter::once(shift(lines[at])).chain(lines[at + 1..].iter().take_while(|l| l.len() - l.trim_start().len() > depth).map(|l| shift(l))).collect();
        assert_eq!(ours.lines().collect::<Vec<_>>(), theirs.iter().map(String::as_str).collect::<Vec<_>>());
    }

    /// Refused: another kind than the start, anything after the element, a comment outside it, TypeScript (no JSX).
    #[test]
    fn a_jsx_fragment_is_one_element() {
        for (src, start) in [("<br />", "jsx_element"), ("<a></a>", "jsx_self_closing_element"), ("<a></a> x", "jsx_element"), ("<a></a><b></b>", "jsx_element"), ("// c\n<a></a>", "jsx_element"), ("<a></a> // c", "jsx_element"), ("x", "jsx_element"), ("", "jsx_element")] {
            assert!(element(src, start).is_err(), "{src:?} from {start}");
        }
        assert!(tape_from_into(b"<a></a>", Dialect::TypeScript, "jsx_element", &mut sink(Dialect::TypeScript)).unwrap().is_err());
        assert!(tape_from_into(b"<a></a>", Dialect::JavaScript, "statement", &mut sink(Dialect::JavaScript)).is_none());
    }
}
