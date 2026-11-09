//! `core.rr`'s base relations (decision 0046), as `docs/rule-syntax/lib/core.rr` states them, read off the same
//! core facts as T's (`super`): what a node's own text and children say, per language.
//!
//! - **Lines** (`Lines`): a document's line starts, by the terminators the tools count: `\n`, `\r\n` and a lone
//!   `\r` for Python (CPython's tokenizer, Ruff), `\n` elsewhere (tree-sitter, ESLint). `line`, `end_line`, `col`,
//!   `src_line` and `line_count` all read it, so they agree.
//! - **Spans** past the nodes: a document's anonymous tokens, its whole text (`source`) and each line
//!   (`src_line`), numbered after its nodes, so `text`, `line`, `end_line`, `col` and `docof` take either.
//! - **Literals** (`literal`): `str`, `strprefix`, `value`, `value_type`, `name`, `comment` and `spread` of a node,
//!   from its kind and its text alone. Python strings decode as the engine's Python values do
//!   (`value::python::str_value`); JavaScript strings as ECMAScript's (`value::ecma`); the others by the common
//!   escapes, as rrlab's extractor does (`docs/rule-syntax/tools/rrlab`).
//! - **`str_part`** (Python): the str constants CPython builds from a literal, pieces split at an f-string's
//!   fields.

use super::{Dialect, Projected, Relation};
use crate::code_rule_dsl::facts::core::Facts;
use std::collections::{BTreeSet, HashMap, HashSet};

/// A document's text may be long: `text` holds a node's when it is at most this many bytes, or the node is
/// token-like (a leaf, a comment, a string literal). Longer texts have no symbol: interning them all is
/// `O(bytes × depth)` per file, and a test on them is the text layer's file scan (decision 0042 §3).
pub(super) const TEXT_INLINE: usize = 256;

/// A language's literal and line rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Family {
    Python,
    Ecma,
    Php,
    Rust,
    /// `.rr` rule files (decision 0061).
    Rr,
    Other,
}

impl Family {
    pub(super) fn of(label: Option<&str>) -> Self {
        match label {
            Some("python") => Self::Python,
            Some("javascript" | "typescript" | "tsx" | "vue") => Self::Ecma,
            Some("php" | "blade") => Self::Php,
            Some("rust") => Self::Rust,
            Some("rr") => Self::Rr,
            _ => Self::Other,
        }
    }
}

/// A document's lines: where each starts and where its text ends (before its terminator).
pub(super) struct Lines {
    starts: Vec<u32>,
    ends: Vec<u32>,
}

impl Lines {
    /// Python str.splitlines: all eleven Unicode line boundaries; CRLF is one.
    /// This is a text view, not the coordinate system of parser spans.
    fn split_python(src: &str) -> impl Iterator<Item = &str> {
        let mut chars = src.char_indices().peekable();
        let mut start = 0;
        std::iter::from_fn(move || {
            while let Some((at, c)) = chars.next() {
                if matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                    let line = &src[start..at];
                    start = at + c.len_utf8();
                    if c == '\r' && chars.peek().is_some_and(|(_, c)| *c == '\n') {
                        let (at, _) = chars.next().unwrap();
                        start = at + 1;
                    }
                    return Some(line);
                }
            }
            if start < src.len() {
                let line = &src[start..];
                start = src.len();
                Some(line)
            } else {
                None
            }
        })
    }
    /// Python ends a line at `\n`, `\r\n` or a lone `\r`; the others at `\n`, a `\r` before it left out of the
    /// line's text. A last line without a terminator counts when it is not empty (Python) or always (the others,
    /// so an empty file has one empty line, as `split('\n')` gives).
    pub(super) fn new(src: &str, python: bool) -> Self {
        let b = src.as_bytes();
        let (mut starts, mut ends) = (vec![0u32], Vec::new());
        let mut i = 0;
        while i < b.len() {
            let end = match b[i] {
                b'\n' => Some((i, i + 1)),
                b'\r' if python && b.get(i + 1) == Some(&b'\n') => Some((i, i + 2)),
                b'\r' if python => Some((i, i + 1)),
                _ => None,
            };
            if let Some((text_end, next)) = end {
                let text_end = if !python && text_end > 0 && b[text_end - 1] == b'\r' && text_end - 1 >= *starts.last().unwrap() as usize {
                    text_end - 1
                } else {
                    text_end
                };
                ends.push(text_end as u32);
                starts.push(next as u32);
                i = next;
            } else {
                i += 1;
            }
        }
        let last = *starts.last().unwrap() as usize;
        if (python && last < b.len()) || (!python && !src.ends_with('\n')) {
            let end = if !python && b.last() == Some(&b'\r') { b.len() - 1 } else { b.len() };
            ends.push(end.max(last) as u32);
        } else {
            starts.pop();
        }
        if starts.is_empty() {
            starts.push(0);
        }
        Lines { starts, ends }
    }
    /// Physical lines, as `src_line` numbers them.
    pub(super) fn count(&self) -> usize {
        self.ends.len()
    }
    /// Line `l`'s text, `[start, end)`, from 1.
    pub(super) fn span(&self, l: usize) -> [u32; 2] {
        [self.starts[l - 1], self.ends[l - 1]]
    }
    /// The 1-based line holding byte `at`.
    pub(super) fn line(&self, at: u32) -> u32 {
        self.starts.partition_point(|&s| s <= at).max(1) as u32
    }
    /// A span's first line, last line (of its last character) and 0-based start column (bytes).
    pub(super) fn place(&self, [start, end]: [u32; 2]) -> (u32, u32, u32) {
        let line = self.line(start);
        let last = if end > start { self.line(end - 1) } else { line };
        (line, last, start - self.starts[line as usize - 1])
    }
}

/// Punctuation `op_at` leaves out: brackets, quotes and separators are structure, not operators.
pub(super) fn is_punct(t: &str) -> bool {
    matches!(t, "(" | ")" | "[" | "]" | "{" | "}" | "," | ";" | ":" | "." | "\"" | "'" | "`")
}

/// Whether a node of `kind` is a definition unit (decision 0083 §0): the unit of identity and change, whose own code
/// `def_digest` hashes. A document's root is one too.
pub(super) fn is_unit(family: Family, kind: &str) -> bool {
    match family {
        Family::Python => matches!(kind, "function_definition" | "class_definition"),
        _ => false,
    }
}

/// A unit's name: the text of its first named child that is an identifier (Python's `def f`, `class C`).
pub(super) fn unit_name<'a>(f: &'a Facts, dialect: &Dialect, id: u32) -> Option<&'a str> {
    f.children(id).iter().copied().find(|&c| f.named(c) && dialect.name(f, c) == "identifier").map(|c| f.text(c))
}

pub(super) fn is_comment(kind: &str) -> bool {
    kind.contains("comment")
}

/// A comment's text without its marker (`#`, `//`, `/* */`, `<!-- -->`, `--`), trimmed.
pub(super) fn comment_body(t: &str) -> &str {
    let t = t.trim_end();
    let body = if let Some(r) = t.strip_prefix("/*") {
        r.strip_suffix("*/").unwrap_or(r)
    } else if let Some(r) = t.strip_prefix("<!--") {
        r.strip_suffix("-->").unwrap_or(r)
    } else {
        ["//", "#", "--"].iter().find_map(|m| t.strip_prefix(m)).unwrap_or(t)
    };
    body.trim()
}

pub(super) fn is_spread(kind: &str) -> bool {
    matches!(kind, "spread_element" | "list_splat" | "dictionary_splat" | "spread_expression" | "variadic_unpacking")
}

/// `name`: an identifier's text, or a dotted chain's without spaces, when it holds only name characters.
pub(super) fn name(kind: &str, text: &str) -> Option<String> {
    let chain = kind.contains("identifier")
        || matches!(kind, "member_expression" | "attribute" | "dotted_name" | "scoped_identifier" | "field_expression" | "qualified_name" | "nested_identifier" | "name");
    if !chain {
        return None;
    }
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        if !(c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | ':' | '\\')) {
            return None;
        }
        s.push(c);
    }
    (!s.is_empty()).then_some(s)
}

/// What a literal node gives: `strprefix`, `str`, and `value` with its `value_type`.
#[derive(Default, Debug, PartialEq)]
pub(super) struct Literal {
    pub(super) prefix: Option<String>,
    pub(super) str: Option<String>,
    pub(super) value: Option<(String, &'static str)>,
}

/// The literal facts of a node of `kind` with source `text`; `interpolated` when it has a part other than
/// constant text (a Python `interpolation`, a PHP `encapsed_string`'s variable).
pub(super) fn literal(family: Family, kind: &str, text: &str, interpolated: bool) -> Literal {
    let mut out = Literal::default();
    match (family, kind) {
        // A `.rr` string reads as the rule engine reads it; a backtick literal's `str` is its body.
        (Family::Rr, "string") => {
            if let Some(v) = crate::code_rule_dsl::logic::rr::string_value(text) {
                out.value = Some((v.clone(), "string"));
                out.str = Some(v);
            }
        }
        (Family::Rr, "code_literal") => out.str = Some(crate::code_rule_dsl::logic::rr::code_body(text).to_owned()),
        (Family::Rr, "number") => out.value = text.parse::<i64>().ok().map(|v| (v.to_string(), "number")),
        (Family::Rr, "float") => out.value = text.parse::<f64>().ok().map(|v| (v.to_string(), "float")),
        (Family::Rr, _) => {}
        (_, "string" | "string_literal" | "raw_string_literal" | "encapsed_string" | "interpreted_string_literal") => {
            let Some((prefix, body)) = string_contents(family, text) else { return out };
            let ty = if family == Family::Python { "str" } else { "string" };
            if !interpolated {
                if !prefix.contains('f') {
                    out.value = Some((body.clone(), ty));
                }
                out.str = Some(body);
            }
            out.prefix = Some(prefix);
        }
        (Family::Ecma, "template_string") => {
            if !text.contains("${") && text.len() >= 2 {
                let body = crate::code_rule_dsl::value::ecma::cooked_text(&text[1..text.len() - 1]).unwrap_or_else(|| unescape(&text[1..text.len() - 1]));
                out.value = Some((body.clone(), "string"));
                out.str = Some(body);
            }
        }
        (Family::Ecma, "number") => out.value = js_number(text).map(|v| (v, "number")),
        (Family::Python, "integer") => {
            let t = text.replace('_', "").to_lowercase();
            out.value = int_value(t.trim_end_matches(['l', 'j'])).map(|v| (v, "int"));
        }
        (Family::Python, "float") => out.value = Some((text.to_owned(), "float")),
        (Family::Rust | Family::Php, "integer_literal" | "integer") => {
            let t: String = text.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>().replace('_', "").to_lowercase();
            // A Rust literal's type suffix (`0x10u8`, `5usize`) is no digit.
            let t = ["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"]
                .iter()
                .find_map(|s| t.strip_suffix(s).filter(|d| !d.is_empty()))
                .unwrap_or(&t);
            out.value = int_value(t).map(|v| (v, "int"));
        }
        (_, "true" | "false") => out.value = Some((kind.to_owned(), "boolean")),
        (Family::Python, "none") => out.value = Some(("None".to_owned(), "NoneType")),
        (Family::Ecma, "null") => out.value = Some(("null".to_owned(), "null")),
        (Family::Ecma, "undefined") => out.value = Some(("undefined".to_owned(), "undefined")),
        (Family::Rust, "boolean_literal") => out.value = Some((text.to_owned(), "bool")),
        _ => {}
    }
    out
}

/// A string literal's prefix letters, lowercased, and its decoded contents.
pub(super) fn string_contents(family: Family, text: &str) -> Option<(String, String)> {
    let p: String = text.chars().take_while(char::is_ascii_alphabetic).collect();
    let rest = &text[p.len()..];
    let prefix = p.to_lowercase();
    let raw = prefix.contains('r');
    if family == Family::Python
        && !prefix.contains('b')
        && let Some(v) = crate::code_rule_dsl::value::python::str_value(text)
    {
        return Some((prefix, v));
    }
    if family == Family::Ecma
        && prefix.is_empty()
        && let Some(units) = crate::code_rule_dsl::value::ecma::string(text)
    {
        return Some((prefix, String::from_utf16_lossy(&units)));
    }
    let rest = if family == Family::Rust && raw { rest.trim_matches('#') } else { rest };
    for q in ["\"\"\"", "'''", "\"", "'", "`"] {
        if rest.len() >= 2 * q.len() && rest.starts_with(q) && rest.ends_with(q) {
            let body = &rest[q.len()..rest.len() - q.len()];
            let v = if raw {
                body.to_owned()
            } else if family == Family::Python {
                py_unescape(body)
            } else {
                unescape(body)
            };
            return Some((prefix, v));
        }
    }
    None
}

/// A Python literal's escapes, as the language reference gives them: unknown escapes keep their backslash, octal up
/// to three digits, `\x`, `\u`, `\U`, a backslash-newline joins lines. For bytes and f-string pieces, which
/// `str_value` does not decode.
pub(super) fn py_unescape(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let hex = |c: &[char], i: usize, n: usize| -> Option<char> {
        if i + n > c.len() {
            return None;
        }
        let h: String = c[i..i + n].iter().collect();
        u32::from_str_radix(&h, 16).ok().and_then(char::from_u32)
    };
    while i < c.len() {
        if c[i] != '\\' || i + 1 >= c.len() {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let e = c[i + 1];
        i += 2;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '\\' | '\'' | '"' => out.push(e),
            '\n' => {}
            '\r' => {
                if c.get(i) == Some(&'\n') {
                    i += 1;
                }
            }
            '0'..='7' => {
                let mut v = e.to_digit(8).unwrap_or(0);
                let mut k = 0;
                while k < 2 && i < c.len() && c[i].is_digit(8) {
                    v = v * 8 + c[i].to_digit(8).unwrap_or(0);
                    i += 1;
                    k += 1;
                }
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'x' | 'u' | 'U' => {
                let n = match e {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                match hex(&c, i, n) {
                    Some(ch) => {
                        out.push(ch);
                        i += n;
                    }
                    None => {
                        out.push('\\');
                        out.push(e);
                    }
                }
            }
            _ => {
                out.push('\\');
                out.push(e);
            }
        }
    }
    out
}

/// The common C-like escapes (`\n`, `\t`, `\xHH`, `\u{…}`, `\uHHHH`, …); an unknown escape gives its character.
pub(super) fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some('u') => {
                let h: String = if it.peek() == Some(&'{') {
                    it.next();
                    it.by_ref().take_while(|c| *c != '}').collect()
                } else {
                    it.by_ref().take(4).collect()
                };
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some('\n') => {}
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// An integer literal's value in decimal (`0x`, `0o`, `0b` read).
pub(super) fn int_value(t: &str) -> Option<String> {
    let (radix, digits) = if let Some(r) = t.strip_prefix("0x") {
        (16, r)
    } else if let Some(r) = t.strip_prefix("0o") {
        (8, r)
    } else if let Some(r) = t.strip_prefix("0b") {
        (2, r)
    } else {
        (10, t)
    };
    u128::from_str_radix(digits, radix).ok().map(|v| v.to_string())
}

/// A JavaScript number literal's value as `Number.prototype.toString` writes it (integers exactly; others in Rust's
/// shortest round-trip form, which agrees for ordinary literals).
pub(super) fn js_number(t: &str) -> Option<String> {
    let t = t.replace('_', "");
    if let Some(b) = t.strip_suffix('n') {
        return int_value(&b.to_lowercase());
    }
    let l = t.to_lowercase();
    if l.starts_with("0x") || l.starts_with("0o") || l.starts_with("0b") {
        return int_value(&l);
    }
    if l.len() > 1 && l.starts_with('0') && l.chars().all(|c| c.is_ascii_digit()) {
        return if l.chars().any(|c| c == '8' || c == '9') {
            Some(l.trim_start_matches('0').to_owned())
        } else {
            u128::from_str_radix(&l[1..], 8).ok().map(|v| v.to_string())
        };
    }
    let f: f64 = l.parse().ok()?;
    Some(if f.fract() == 0.0 && f.abs() < 1e21 { format!("{}", f as i128) } else { format!("{f}") })
}

/// Decoded parts and their source byte offsets, relative to the part's literal token.
#[cfg(test)]
pub(super) fn str_parts_with_offsets<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8, usize)> {
    super::python_parts::with_offsets(parts)
}

/// `str_part(lit, i, v, at, spec)`: the str constants CPython builds from Python literal `lit` (a `string`, or a
/// `concatenated_string`'s parts `parts`, each with its node): plain runs join across adjacent literals; an
/// f-string's fields split them; a format spec's literal runs are pieces of their own (`spec` 1). Bytes and
/// t-strings give none. Returns (v, at, spec) per piece, in order.
#[cfg(test)]
pub(super) fn str_parts<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8)> {
    super::python_parts::with_offsets(parts).into_iter().map(|(v, n, spec, _)| (v, n, spec)).collect()
}

/// A shared native grammar verdict. Raw text projections do not request it.
/// Without `python-syntax-error` (the browser build), Python's verdict is tree-sitter's, as for the other languages:
/// it misses what tree-sitter accepts and CPython does not (Python 2's `print "x"`).
pub(super) fn verdict(f: &Facts, dialect: &Dialect) -> Option<(u32, String)> {
    #[cfg(feature = "python-syntax-error")]
    if dialect.family == Family::Python {
        return f.python_syntax.get_or_init(|| super::python_syntax::error(&f.source)).clone();
    }
    // A `.rr` file's error is the one running it gives (`logic::rr::syntax_error`).
    if dialect.family == Family::Rr {
        return f.syntax_errors.then(|| crate::code_rule_dsl::logic::rr::syntax_error(&f.source)).flatten().map(|(line, message)| (line as u32, message));
    }
    first_error(f, &Lines::new(&f.source, dialect.family == Family::Python), &|id| dialect.name(f, id))
}

/// The first error of a document, in preorder: its line and message (`syntax_error/3`). tree-sitter's ERROR and
/// MISSING nodes. Python uses its demanded, cached native grammar verdict instead.
pub(super) fn first_error(f: &Facts, lines: &Lines, kind_name: &dyn Fn(u32) -> &'static str) -> Option<(u32, String)> {
    if !f.syntax_errors {
        return None;
    }
    (0..f.len() as u32).find_map(|id| {
        let flags = f.flags[id as usize];
        let (missing, error) = (flags & 4 != 0, flags & 8 != 0);
        (missing || error).then(|| {
            let msg = if missing { format!("missing {}", kind_name(id)) } else { "syntax error".to_owned() };
            (lines.line(f.start[id as usize]), msg)
        })
    })
}

/// `core.rr`'s relations of a batch, as rows with string indexes (`Batch::build`'s `string`) until the batch's
/// strings are interned.
#[derive(Default)]
pub(super) struct CoreRows {
    op: Vec<[i64; 2]>,
    op_at: Vec<[i64; 3]>,
    text: Vec<[i64; 2]>,
    name: Vec<[i64; 2]>,
    str: Vec<[i64; 2]>,
    strprefix: Vec<[i64; 2]>,
    value: Vec<[i64; 2]>,
    value_type: HashSet<[i64; 2]>,
    comment: Vec<[i64; 2]>,
    spread: Vec<i64>,
    next_sibling: Vec<[i64; 2]>,
    syntax_error: Vec<[i64; 3]>,
    source: Vec<[i64; 2]>,
    src_line: Vec<[i64; 3]>,
    split_line: Vec<[i64; 3]>,
    lf_line: Vec<[i64; 3]>,
    line_count: Vec<[i64; 2]>,
    str_part: Vec<[i64; 5]>,
    str_part_start: Vec<[i64; 4]>,
    str_part_word: Vec<[i64; 5]>,
    first_token: Vec<[i64; 2]>,
    last_token: Vec<[i64; 2]>,
    token_before: Vec<[i64; 2]>,
    token_after: Vec<[i64; 2]>,
    tree_digest: Vec<[i64; 2]>,
    /// `def_digest(n, h)`: each definition unit's own-content hash (`digests`).
    def_digest: Vec<[i64; 2]>,
    rx_span: Vec<[i64; 5]>,
    ast_match: Vec<[i64; 4]>,
    /// A pattern's matches in a node's or capture's text, by pattern and text, for the batch (decision 0081): each
    /// distinct text is scanned once. Each match's groups as (match, group, start, end), offsets in the text.
    rx_memo: HashMap<(usize, Box<str>), std::sync::Arc<[(u32, u32, u32, u32)]>>,
}

/// Every match of `re` in `text`, each named group (and the whole match, group 0) that takes part: (match, group,
/// start, end), byte offsets in `text`.
fn scan(re: &regex::Regex, names: &[Option<&str>], text: &str) -> std::sync::Arc<[(u32, u32, u32, u32)]> {
    let mut out = Vec::new();
    for (j, caps) in re.captures_iter(text).enumerate() {
        for (g, name) in names.iter().enumerate() {
            if name.is_none() {
                continue;
            }
            if let Some(m) = caps.get(g) {
                out.push((j as u32, g as u32, m.start() as u32, m.end() as u32));
            }
        }
    }
    out.into()
}

/// A pattern `rx_span` runs: over which spans, its text, its regex, and over a document's source or lines its
/// block program (`transducer::packed::regex_block`), which marks where a match can end.
pub(super) type SpanPattern<'a> = (crate::code_rule_dsl::logic::front::SpanIn, &'a str, regex::Regex, Option<crate::code_rule_dsl::transducer::packed::regex_block::SpanPrefilter>);

/// Texts past this are scanned where they are, not kept in `CoreRows::rx_memo`.
const RX_MEMO_TEXT: usize = 4096;

impl CoreRows {
    /// Node `local`'s rows: what its anonymous children, its kind and its own text give.
    pub(super) fn node(&mut self, p: &Projected, dialect: &Dialect, local: u32, string: &mut dyn FnMut(&str) -> usize, wants_text: bool, wants_parts: bool, wants_part_starts: bool, wants_words: bool) {
        let f = &*p.facts;
        let id = p.core[local as usize];
        let n = p.global(local);
        let kind = dialect.name(f, id);
        let text = f.text(id);
        let children = f.children(id);
        // `op`: every anonymous child; `op_at`: those that are no punctuation, in order, Python's two-word
        // operators as one ("not in", "is not"), which `op` has too.
        let mut ops: Vec<String> = Vec::new();
        for &c in children {
            if f.named(c) {
                continue;
            }
            let t = dialect.name(f, c);
            self.op.push([n, string(t) as i64]);
            if !is_punct(t) {
                let two = dialect.family == Family::Python && matches!((ops.last().map(String::as_str), t), (Some("not"), "in") | (Some("is"), "not"));
                match ops.last_mut() {
                    Some(last) if two => *last = format!("{last} {t}"),
                    _ => ops.push(t.to_owned()),
                }
            }
        }
        for (i, t) in ops.iter().enumerate() {
            self.op_at.push([n, i as i64, string(t) as i64]);
            if t.contains(' ') {
                self.op.push([n, string(t) as i64]);
            }
        }
        let mut prev = None;
        for c in p.children(local) {
            if let Some(a) = prev {
                self.next_sibling.push([p.global(a), p.global(c)]);
            }
            prev = Some(c);
        }
        let mut token_like = children.is_empty();
        if is_comment(kind) {
            self.comment.push([n, string(comment_body(text)) as i64]);
            token_like = true;
        } else {
            if is_spread(kind) {
                self.spread.push(n);
            }
            if let Some(s) = name(kind, text) {
                self.name.push([n, string(&s) as i64]);
            } else if dialect.family == Family::Rr && kind == "variable" {
                self.name.push([n, string(text) as i64]);
            }
            let named_child = |k: &dyn Fn(&str) -> bool| children.iter().any(|&c| f.named(c) && k(dialect.name(f, c)));
            let interpolated = match (dialect.family, kind) {
                (Family::Python, _) => named_child(&|k| k == "interpolation"),
                (Family::Php, "encapsed_string") => named_child(&|k| !matches!(k, "string_content" | "escape_sequence")),
                _ => false,
            };
            let lit = literal(dialect.family, kind, text, interpolated);
            if let Some(prefix) = &lit.prefix {
                self.strprefix.push([n, string(prefix) as i64]);
                token_like = true;
            }
            if let Some(v) = &lit.str {
                self.str.push([n, string(v) as i64]);
            }
            if let Some((v, ty)) = &lit.value {
                let v = string(v) as i64;
                self.value.push([n, v]);
                self.value_type.insert([v, string(ty) as i64]);
            }
            if dialect.family == Family::Python
                && (wants_parts || wants_part_starts || wants_words)
                && (kind == "concatenated_string" || kind == "string" && f.parent(id).is_none_or(|q| dialect.name(f, q) != "concatenated_string"))
            {
                // A word-only reader also gets coordinates from this pass. It never interns full values.
                let word_parts = if wants_words { super::python_parts::from_tape(p, dialect, local, true) } else { Vec::new() };
                if wants_words {
                    for (i, (v, at, spec, _)) in word_parts.iter().enumerate() {
                        for word in v.split(|c: char| !crate::code_rule_dsl::value::python::words::is_word(u32::from(c))).filter(|s| !s.is_empty()) {
                            self.str_part_word.push([n, i as i64, string(word) as i64, *at, i64::from(*spec)]);
                        }
                    }
                }
                let position_parts = if wants_parts || wants_part_starts && !wants_words { super::python_parts::from_tape(p, dialect, local, false) }
                    else if wants_part_starts { word_parts } else { Vec::new() };
                for (i, (v, at, spec, offset)) in position_parts.into_iter().enumerate() {
                    if wants_parts {
                        self.str_part.push([n, i as i64, string(&v) as i64, at, i64::from(spec)]);
                    }
                    if wants_part_starts {
                        // `at` is a node id: its low bits are a preorder index only under preorder ids.
                        let Some(at_local) = p.local_of(at as u32) else { continue };
                        let absolute = f.start[p.core[at_local as usize] as usize] + offset as u32;
                        let (line, _, col) = p.extra.as_ref().expect("core spans").lines.place([absolute, absolute]);
                        self.str_part_start.push([n, i as i64, i64::from(line), i64::from(col)]);
                    }
                }
            }
        }
        if wants_text && (token_like || text.len() <= TEXT_INLINE) {
            self.text.push([n, string(text) as i64]);
        }
    }

    /// A withheld document's one row: its first error (`verdict`).
    pub(super) fn withheld(&mut self, p: &Projected, (l, msg): &(u32, String), string: &mut dyn FnMut(&str) -> usize) {
        self.syntax_error.push([i64::from(p.key), i64::from(*l), string(msg) as i64]);
    }

    /// A document's rows: its source-text spans (a file's own, `own_text`; a guest's text is part of its file's), and
    /// its tokens.
    pub(super) fn document(&mut self, p: &Projected, dialect: &Dialect, string: &mut dyn FnMut(&str) -> usize, wants_text: bool, wants_tokens: bool, wants_split_lines: bool, wants_lf_lines: bool, wants_source_text: bool, wants_line_text: bool, own_text: bool) {
        let f = &*p.facts;
        let x = p.extra.as_ref().expect("core.rr's spans");
        let key = i64::from(p.key);
        if own_text {
            let source = p.len() + x.tokens;
            self.source.push([key, p.global(source)]);
            for l in 1..=x.lines.count() {
                self.src_line.push([key, l as i64, p.global(source + l as u32)]);
            }
            self.line_count.push([key, x.lines.count() as i64]);
            if wants_split_lines {
                for (i, line) in Lines::split_python(&f.source).enumerate() {
                    self.split_line.push([key, (i + 1) as i64, string(line) as i64]);
                }
            }
            if wants_lf_lines {
                for (i, line) in f.source.split('\n').enumerate() {
                    self.lf_line.push([key, (i + 1) as i64, string(line) as i64]);
                }
            }
        }
        if wants_text {
            for (i, &[a, b]) in x.spans.iter().enumerate() {
                let requested_container = (wants_source_text && i == x.tokens as usize)
                    || (wants_line_text && i > x.tokens as usize && i <= x.tokens as usize + x.lines.count());
                if requested_container || i < x.tokens as usize || i >= x.matched as usize || (b - a) as usize <= TEXT_INLINE {
                    self.text.push([p.global(p.len() + i as u32), string(&f.source[a as usize..b as usize]) as i64]);
                }
            }
        }
        if !wants_tokens {
            return;
        }
        // Tokens: the leaves, named or anonymous, other than comments, in source order (ESLint's token store).
        let mut toks: Vec<(u32, u32, i64)> = (0..p.len())
            .filter_map(|l| {
                let id = p.core[l as usize];
                let [a, b] = p.span(l);
                (f.children(id).is_empty() && b > a && !is_comment(dialect.name(f, id))).then(|| (a, b, p.global(l)))
            })
            .collect();
        toks.extend((0..x.tokens).map(|i| {
            let [a, b] = x.spans[i as usize];
            (a, b, p.global(p.len() + i))
        }));
        toks.sort_unstable();
        let starts: Vec<u32> = toks.iter().map(|t| t.0).collect();
        let ends: Vec<u32> = toks.iter().map(|t| t.1).collect();
        for l in 0..p.len() + x.tokens {
            let [a, b] = p.span(l);
            let id = p.global(l);
            let first = starts.partition_point(|&s| s < a);
            if first < toks.len() && toks[first].1 <= b {
                let last = ends.partition_point(|&e| e <= b) - 1;
                self.first_token.push([id, toks[first].2]);
                self.last_token.push([id, toks[last].2]);
            }
            let k = ends.partition_point(|&e| e <= a);
            if k > 0 && toks[k - 1].2 != id {
                self.token_before.push([id, toks[k - 1].2]);
            }
            let j = starts.partition_point(|&s| s < b);
            if j < toks.len() && toks[j].2 != id {
                self.token_after.push([id, toks[j].2]);
            }
        }
    }

    /// `rx_span(p, c, j, g, m)`: group `g` of match `j` (from 0) of regex `p` in span `c` is span `m`, made here past
    /// the lines (`Extra::matched`). Group "" is the whole match; a group that takes no part in a match has no row.
    /// Matches do not overlap, as `match_all` says; a `match` pattern is anchored (`rr::lower`), so it has one.
    ///
    /// A span `c` is a document's `source`, one of its lines (`src_line`), a comment, a node of a kind, or a span an
    /// earlier pattern's group captured (`SpanIn`). The source is scanned once per pattern (decision 0042 §3). A
    /// line's, node's or capture's text is scanned once per distinct text in the batch (0042 §2, `rx_memo`), and its
    /// matches placed at each span holding it: nodes of one kind seldom nest, and a capture lies inside its text.
    pub(super) fn matches(&mut self, p: &mut Projected, dialect: &Dialect, patterns: &[SpanPattern<'_>], ends: &[Option<Vec<u64>>], string: &mut dyn FnMut(&str) -> usize) {
        use crate::code_rule_dsl::logic::front::SpanIn;
        let facts = std::sync::Arc::clone(&p.facts);
        let src = facts.source.as_str();
        let base = p.len();
        // Nodes by kind, for the kinds a pattern runs over (comments by the dialect's test): their own spans.
        let kinds: BTreeSet<&str> = patterns.iter().filter_map(|(w, _, _, _)| if let SpanIn::Kind(k) = w { Some(k.as_str()) } else { None }).collect();
        let wants_comments = patterns.iter().any(|(w, _, _, _)| *w == SpanIn::Comment);
        let mut of_kind: HashMap<&str, Vec<([u32; 2], u32)>> = HashMap::new();
        let mut comments: Vec<([u32; 2], u32)> = Vec::new();
        if wants_comments || !kinds.is_empty() {
            for l in 0..base {
                let name = dialect.name(&facts, p.core[l as usize]);
                if wants_comments && is_comment(name) {
                    comments.push((p.span(l), l));
                }
                if let Some(k) = kinds.get(name) {
                    of_kind.entry(k).or_default().push((p.span(l), l));
                }
            }
        }
        let x = p.extra.as_mut().expect("core.rr's spans");
        let source = x.tokens;
        // Spans each (pattern, group) captured in this document: the containers of a `SpanIn::Capture`.
        let mut captured: HashMap<(&str, &str), Vec<([u32; 2], u32)>> = HashMap::new();
        // A capture's container is made by an earlier pattern: patterns run once theirs has.
        // Patterns whose containers wait on each other (a cycle of captures) run in order, with what is made by then.
        let mut done = vec![false; patterns.len()];
        let mut force = false;
        // Bytes scanned, reported as profile work.
        let mut scanned = 0u64;
        loop {
            let mut progressed = false;
            for (i, (within, pattern, re, prefilter)) in patterns.iter().enumerate() {
                if done[i] {
                    continue;
                }
                if !force
                    && let SpanIn::Capture(from, _) = within
                    && patterns.iter().enumerate().any(|(k, (_, q, _, _))| !done[k] && k != i && *q == from.as_str())
                {
                    continue;
                }
                done[i] = true;
                progressed = true;
                let pat = string(pattern) as i64;
                let (containers, memo): (Vec<([u32; 2], u32)>, bool) = match within {
                    SpanIn::Source => (vec![(x.spans[source as usize], base + source)], false),
                    SpanIn::Line => ((1..=x.lines.count() as u32).map(|l| (x.spans[(source + l) as usize], base + source + l)).collect(), true),
                    SpanIn::Comment => (comments.clone(), true),
                    SpanIn::Kind(k) => (of_kind.get(k.as_str()).cloned().unwrap_or_default(), true),
                    SpanIn::Capture(from, g) => (captured.get(&(from.as_str(), g.as_str())).cloned().unwrap_or_default(), true),
                };
                let names: Vec<Option<&str>> = re.capture_names().enumerate().map(|(g, n)| if g == 0 { Some("") } else { n }).collect();
                // The bytes a match can end after (decision 0081 §3), from the batch's run of the block programs: a
                // source or line where none can is not scanned.
                let ends = match (&ends[i], prefilter) {
                    (Some(e), _) => Some(std::borrow::Cow::Borrowed(e.as_slice())),
                    (None, Some(f)) => Some(std::borrow::Cow::Owned(f.ends(src.as_bytes()))),
                    (None, None) => None,
                };
                for ([a, b], c) in containers {
                    if let Some(ends) = &ends
                        && !crate::code_rule_dsl::transducer::packed::regex_block::SpanPrefilter::any_in(ends, a as usize, b as usize)
                    {
                        continue;
                    }
                    let cid = Projected::id(p.key, &p.defs, c);
                    let text = &src[a as usize..b as usize];
                    let found: std::sync::Arc<[(u32, u32, u32, u32)]> = match memo && text.len() <= RX_MEMO_TEXT {
                        true => match self.rx_memo.get(&(pat as usize, Box::from(text))) {
                            Some(f) => std::sync::Arc::clone(f),
                            None => {
                                let f = { scanned += text.len() as u64; scan(re, &names, text) };
                                self.rx_memo.insert((pat as usize, Box::from(text)), std::sync::Arc::clone(&f));
                                f
                            }
                        },
                        false => { scanned += text.len() as u64; scan(re, &names, text) },
                    };
                    for &(j, g, s0, e0) in found.iter() {
                        let name = names[g as usize].expect("a named group");
                        let id = base + x.spans.len() as u32;
                        x.spans.push([a + s0, a + e0]);
                        self.rx_span.push([pat, cid, i64::from(j), string(name) as i64, Projected::id(p.key, &p.defs, id)]);
                        captured.entry((pattern, name)).or_default().push(([a + s0, a + e0], id));
                    }
                }
            }
            if done.iter().all(|d| *d) {
                break;
            }
            force = !progressed;
        }
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, scanned);
    }

    /// `ast_match(p, n, g, c)`: AST pattern `p` (``n ~ `p` ``, `rr::lower`) matches node `n`, and its metavariable
    /// `g` captured node `c`; capture "" is `n` itself. ast-grep matches each pattern over the document's tree once;
    /// a match is the projected node of its kind and byte range (the outermost, when nodes share both).
    pub(super) fn patterns(&mut self, p: &Projected, dialect: &Dialect, lang: crate::code_rule_dsl::language::GrammarDialect, patterns: &[(&str, ast_grep_core::matcher::Pattern)], string: &mut dyn FnMut(&str) -> usize) {
        if patterns.is_empty() {
            return;
        }
        let f = &*p.facts;
        let mut at: std::collections::HashMap<(u32, u32, &str), u32> = std::collections::HashMap::new();
        for local in 0..p.len() {
            let id = p.core[local as usize];
            at.entry((f.start[id as usize], f.end[id as usize], dialect.name(f, id))).or_insert(local);
        }
        let tree = crate::code_rule_dsl::language::adapter::ast_grep(lang, &f.source);
        let node_of = |n: &ast_grep_core::Node<'_, _>| {
            let r = n.range();
            at.get(&(r.start as u32, r.end as u32, &*n.kind())).map(|&local| p.global(local))
        };
        let empty = string("") as i64;
        for (text, pattern) in patterns {
            let pat = string(text) as i64;
            let names = crate::code_rule_dsl::logic::rr::single_metavars(text);
            for m in tree.root().find_all(pattern) {
                let Some(n) = node_of(&m) else { continue };
                let captures: Option<Vec<(i64, i64)>> = names
                    .iter()
                    .map(|name| m.get_env().get_match(name).and_then(|c| node_of(c)).map(|c| (string(name) as i64, c)))
                    .collect();
                let Some(captures) = captures else { continue };
                self.ast_match.push([pat, n, empty, n]);
                self.ast_match.extend(captures.into_iter().map(|(g, c)| [pat, n, g, c]));
            }
        }
    }

    /// `tree_digest(n, h)` (`@tree_digest(n)`): a structural hash of n's subtree, without
    /// positions. This is not CPython ast.dump serialization. A node hashes its kind, and its children's field and hash in order; a leaf, or a literal with a
    /// static value, its text or value. Comments and punctuation are left out, and a parenthesized expression is
    /// the expression inside, so `(a)` and `a`, `'x'` and `"x"`, or `0x10` and `16`, hash the same. Made when a
    /// program reads it: every node's hash is a 64-character symbol.
    ///
    /// `def_digest(n, h)` (decision 0083 §0): one row per definition unit, the document's root and each node `is_unit`
    /// names, hashing the unit's own code. It is `tree_digest`'s hash with each nested unit replaced by its kind and
    /// name, so editing a method's body changes the method's digest and not its class's or its module's.
    pub(super) fn digests(&mut self, p: &Projected, dialect: &Dialect, string: &mut dyn FnMut(&str) -> usize, trees: bool, units: bool) {
        use sha2::{Digest, Sha256};
        let f = &*p.facts;
        // Children have greater ids than their parent (preorder), so one pass from the end sees them first.
        let mut hash: Vec<[u8; 32]> = vec![[0; 32]; f.len()];
        // The same hash, each nested unit standing for its kind and name.
        let mut own: Vec<[u8; 32]> = vec![[0; 32]; f.len()];
        let unit = |id: u32| is_unit(dialect.family, dialect.name(f, id));
        let placeholder = |id: u32| -> [u8; 32] {
            let mut h = Sha256::new();
            h.update(b"unit\0");
            h.update(dialect.name(f, id).as_bytes());
            h.update([0]);
            h.update(unit_name(f, dialect, id).unwrap_or_default().as_bytes());
            h.finalize().into()
        };
        for id in (0..f.len() as u32).rev() {
            let kind = dialect.name(f, id);
            let kept: Vec<u32> = f
                .children(id)
                .iter()
                .copied()
                .filter(|&c| {
                    let k = dialect.name(f, c);
                    !is_comment(k) && (f.named(c) || !is_punct(k))
                })
                .collect();
            if kind == "parenthesized_expression" && kept.len() == 1 {
                hash[id as usize] = hash[kept[0] as usize];
                own[id as usize] = own[kept[0] as usize];
                continue;
            }
            let interpolated = dialect.family == Family::Python
                && kept.iter().any(|&c| f.named(c) && dialect.name(f, c) == "interpolation");
            let (mut h, mut o) = (Sha256::new(), Sha256::new());
            for x in [&mut h, &mut o] {
                x.update(kind.as_bytes());
                x.update([0]);
            }
            match literal(dialect.family, kind, f.text(id), interpolated).value {
                Some((value, ty)) => {
                    for x in [&mut h, &mut o] {
                        x.update(b"=");
                        x.update(ty.as_bytes());
                        x.update([0]);
                        x.update(value.as_bytes());
                    }
                }
                None if kept.is_empty() => {
                    for x in [&mut h, &mut o] {
                        x.update(b"'");
                        x.update(f.text(id).as_bytes());
                    }
                }
                None => {
                    for &c in &kept {
                        h.update(f.field[c as usize].to_le_bytes());
                        h.update(hash[c as usize]);
                        o.update(f.field[c as usize].to_le_bytes());
                        o.update(if units && unit(c) { placeholder(c) } else { own[c as usize] });
                    }
                }
            }
            hash[id as usize] = h.finalize().into();
            own[id as usize] = o.finalize().into();
        }
        let hex = |h: &[u8; 32]| -> String { h.iter().map(|b| format!("{b:02x}")).collect() };
        for local in 0..p.len() {
            let id = p.core[local as usize];
            if trees {
                self.tree_digest.push([p.global(local), string(&hex(&hash[id as usize])) as i64]);
            }
            if units && (local == 0 || unit(id)) {
                self.def_digest.push([p.global(local), string(&hex(&own[id as usize])) as i64]);
            }
        }
    }

    /// The relations, the strings' indexes made atoms by `atom`.
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        fn make<const N: usize>(columns: &[&str], rows: &[[i64; N]], sym: [bool; N], atom: &dyn Fn(i64) -> i64) -> Relation {
            let mut r = Relation::new(columns);
            let mut row = [0i64; N];
            for v in rows {
                for i in 0..N {
                    row[i] = if sym[i] { atom(v[i]) } else { v[i] };
                }
                r.push(&row);
            }
            r
        }
        let node_sym = |rows: &[[i64; 2]]| make(&["n", "s"], rows, [false, true], atom);
        let pair = |columns: &[&str; 2], rows: &[[i64; 2]]| make(columns, rows, [false, false], atom);
        let mut value_type: Vec<[i64; 2]> = self.value_type.into_iter().collect();
        value_type.sort_unstable();
        let spread: Vec<[i64; 1]> = self.spread.iter().map(|&n| [n]).collect();
        vec![
            ("op", node_sym(&self.op)),
            ("op_at", make(&["n", "i", "s"], &self.op_at, [false, false, true], atom)),
            ("text", node_sym(&self.text)),
            ("name", node_sym(&self.name)),
            ("str", node_sym(&self.str)),
            ("strprefix", node_sym(&self.strprefix)),
            ("value", node_sym(&self.value)),
            ("static_value", node_sym(&self.value)),
            ("value_type", make(&["v", "t"], &value_type, [true, true], atom)),
            ("comment", node_sym(&self.comment)),
            ("spread", make(&["n"], &spread, [false], atom)),
            ("next_sibling", pair(&["a", "b"], &self.next_sibling)),
            ("syntax_error", make(&["d", "l", "msg"], &self.syntax_error, [false, false, true], atom)),
            ("source", pair(&["d", "sp"], &self.source)),
            ("src_line", make(&["d", "l", "sp"], &self.src_line, [false, false, false], atom)),
            ("split_line", make(&["d", "l", "t"], &self.split_line, [false, false, true], atom)),
            ("lf_line", make(&["d", "l", "t"], &self.lf_line, [false, false, true], atom)),
            ("line_count", pair(&["d", "k"], &self.line_count)),
            ("str_part", make(&["lit", "i", "v", "at", "spec"], &self.str_part, [false, false, true, false, false], atom)),
            ("str_part_word", make(&["lit", "i", "w", "at", "spec"], &self.str_part_word, [false, false, true, false, false], atom)),
            ("str_part_start", make(&["lit", "i", "l", "c"], &self.str_part_start, [false; 4], atom)),
            ("first_token", pair(&["n", "t"], &self.first_token)),
            ("last_token", pair(&["n", "t"], &self.last_token)),
            ("token_before", pair(&["x", "t"], &self.token_before)),
            ("token_after", pair(&["x", "t"], &self.token_after)),
            ("tree_digest", node_sym(&self.tree_digest)),
            ("def_digest", node_sym(&self.def_digest)),
            ("rx_span", make(&["p", "c", "j", "g", "m"], &self.rx_span, [true, false, false, true, false], atom)),
            ("ast_match", make(&["p", "n", "g", "c"], &self.ast_match, [true, false, true, false], atom)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python's verdict is CPython's grammar's: Python 2's `print "x"`, which tree-sitter accepts, is an error; a
    /// `match` statement is not; the line is the first error's.
    #[test]
    #[cfg(feature = "python-syntax-error")]
    fn python_syntax_errors_follow_cpythons_grammar() {
        use super::super::python_syntax::error as python_error;
        assert_eq!(python_error("x = 1\n"), None);
        assert_eq!(python_error("match x:\n    case 1:\n        pass\n"), None);
        assert_eq!(python_error("y = 2\nprint \"x\"\n").map(|e| e.0), Some(2));
        assert_eq!(python_error("y = 2\r\nz = 3\rdef (:\n").map(|e| e.0), Some(3), "lines as src_line counts them");
    }

    #[test]
    fn lines_follow_each_languages_terminators() {
        let py = Lines::new("a\r\nb\rc\n\nd", true);
        assert_eq!(py.count(), 5);
        assert_eq!((1..=5).map(|l| py.span(l)).collect::<Vec<_>>(), [[0, 1], [3, 4], [5, 6], [7, 7], [8, 9]]);
        assert_eq!(py.line(5), 3);
        let js = Lines::new("a\r\nb\rc\n", false);
        assert_eq!(js.count(), 2);
        assert_eq!(js.span(1), [0, 1]);
        assert_eq!(js.span(2), [3, 6]);
        assert_eq!(Lines::new("", false).count(), 1);
        assert_eq!(Lines::new("", true).count(), 0);
        // A node ending at a line's start ends on the line before.
        assert_eq!(js.place([0, 3]), (1, 1, 0));
        assert_eq!(js.place([4, 5]), (2, 2, 1));
    }

    #[test]
    fn literals_decode_per_language() {
        let lit = |f, k, t| literal(f, k, t, false);
        assert_eq!(lit(Family::Python, "string", r#""a\nb""#).str.as_deref(), Some("a\nb"));
        assert_eq!(lit(Family::Python, "string", r#"r"a\nb""#).str.as_deref(), Some("a\\nb"));
        let f = lit(Family::Python, "string", r#"F"x""#);
        assert_eq!((f.prefix.as_deref(), f.str.as_deref(), f.value), (Some("f"), Some("x"), None));
        assert_eq!(literal(Family::Python, "string", r#"f"x{y}""#, true).str, None);
        assert_eq!(lit(Family::Python, "integer", "0xff").value, Some(("255".to_owned(), "int")));
        assert_eq!(lit(Family::Ecma, "string", r"'aA'").str.as_deref(), Some("aA"));
        assert_eq!(lit(Family::Ecma, "template_string", "`a\\tb`").value, Some(("a\tb".to_owned(), "string")));
        assert_eq!(lit(Family::Ecma, "number", "1_000").value, Some(("1000".to_owned(), "number")));
        assert_eq!(lit(Family::Rust, "raw_string_literal", r##"r#"a\n"#"##).str.as_deref(), Some("a\\n"));
        assert_eq!(lit(Family::Rust, "integer_literal", "0x10u8").value, Some(("16".to_owned(), "int")));
        assert_eq!(lit(Family::Php, "encapsed_string", r#""a\tb""#).str.as_deref(), Some("a\tb"));
        assert_eq!(name("attribute", "a . b"), Some("a.b".to_owned()));
        assert_eq!(name("member_expression", "a(b).c"), None);
        assert_eq!(comment_body("# noqa: E501 "), "noqa: E501");
        assert_eq!(comment_body("/* x */"), "x");
    }

    #[test]
    fn decoded_string_constants_keep_precise_byte_starts() {
        let parts = str_parts_with_offsets(&[(0, "f\"\"\"json\n{a}\njson\"\"\"")]);
        assert_eq!(parts.iter().map(|p| p.3).collect::<Vec<_>>(), [4, 12]);
        let parts = str_parts_with_offsets(&[(0, "f'λ{x}μ'")]);
        assert_eq!(parts.iter().map(|p| p.3).collect::<Vec<_>>(), [2, 7]);
        assert_eq!(str_parts_with_offsets(&[(0, "'plain'")])[0].3, 0);
    }

    #[test]
    fn str_parts_follow_cpython() {
        let parts = |lits: &[&str]| str_parts(&lits.iter().enumerate().map(|(i, t)| (i, *t)).collect::<Vec<_>>());
        assert_eq!(parts(&["'a'", "\"b\\n\""]), [("ab\n".to_owned(), 0, 0)]);
        assert_eq!(parts(&["f'x{y}z{{'"]), [("x".to_owned(), 0, 0), ("z{".to_owned(), 0, 0)]);
        assert_eq!(parts(&["f'{x:>{w}d}'"]), [(">".to_owned(), 0, 1), ("d".to_owned(), 0, 1)]);
        assert_eq!(parts(&["f'{x=}'"]), [("x=".to_owned(), 0, 0)]);
        assert!(parts(&["b'x'"]).is_empty());
    }
}
