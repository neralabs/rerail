//! `match-regex`: a rule file's regexes (decision 0495 §2), each a `regex` program of its own: the first argument of
//! a `match(re, s)` atom, where it is a plain string.
//!
//! ```text
//! bad(t) :- name(t), match("(?i)select .*", t).     a regex program: `(?i)select .*`, at the string's content
//! bad(t) :- name(t), match(R, t), re(R).            none: the regex is a variable's
//! ```
//!
//! A program's text is the string's value as the rule file's language reads it (`rr_syntax::unescape`): a stretch
//! with no escape is copied byte for byte, and an escape the language decodes (`\"`, `\\`, `\n`, `\t`, `\r`, `\u…`)
//! is written over its bytes. Any other backslash is kept as written, so `"\d+\.py"` is the regex `\d+\.py`, each
//! byte the file's. The program stands at the string, around its content.

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;

/// The embedder of `Via::MatchRegex`.
pub struct MatchRegex;

impl Embedder for MatchRegex {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let mut programs = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "atom"
                && let Some(string) = regex_argument(node, source)
                && let Some(program) = program(string, source, budget)
            {
                programs.push(program);
                continue;
            }
            let children = node.named_children().collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        programs
    }

    fn per_place(&self) -> bool {
        true
    }
}

/// The plain string a `match` atom's first argument is.
fn regex_argument<'t>(atom: HostNode<'t>, source: &str) -> Option<HostNode<'t>> {
    let relation = atom.named_child(0)?;
    if relation.kind() != "identifier" || &source[relation.byte_range()] != "match" {
        return None;
    }
    let arguments = atom.named_children().find(|c| c.kind() == "argument_list")?;
    let first = arguments.named_child(0)?;
    (first.kind() == "string").then_some(first)
}

/// The regex program of `string`: its value, each stretch mapped to its bytes.
fn program(string: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
    let range = string.byte_range();
    let quoted = &source[range.clone()];
    if quoted.len() < 2 || !quoted.starts_with('"') || !quoted.ends_with('"') {
        return None;
    }
    let (start, end) = (range.start + 1, range.end - 1);
    let mut text = String::new();
    let mut segments = SegmentMap::new();
    let mut copied = start;
    let mut i = start;
    while i < end {
        let bytes = source.as_bytes();
        if bytes[i] != b'\\' || i + 1 >= end || !matches!(bytes[i + 1], b'"' | b'\\' | b'n' | b't' | b'r' | b'u') {
            i += if bytes[i] == b'\\' && i + 1 < end { 2 } else { 1 };
            continue;
        }
        // An escape the language decodes: the copied stretch before it, then its value over its bytes.
        let len = escape_len(&source[i..end]);
        if copied < i {
            let at = text.len();
            text.push_str(&source[copied..i]);
            segments.push(at, text.len(), copied..i, true, false);
        }
        let value = rerail_rr_syntax::syntax::unescape(&source[i..i + len]).ok()?;
        let at = text.len();
        text.push_str(&value);
        segments.push(at, text.len(), i..i + len, false, false);
        i += len;
        copied = i;
    }
    if copied < end {
        let at = text.len();
        text.push_str(&source[copied..end]);
        segments.push(at, text.len(), copied..end, true, false);
    }
    if text.is_empty() {
        return None;
    }
    segments.index();
    let parse = super::Parse::read_program(GrammarDialect::Regex, &text, &segments, budget)?;
    Some(Program { dialect: GrammarDialect::Regex, parse, text, segments, host_errors: Vec::new(), at: Some(range) })
}

/// The bytes of the decoded escape at the start of `rest` (`\n`, `\u{1F600}`, `é`, a surrogate pair's two).
fn escape_len(rest: &str) -> usize {
    let b = rest.as_bytes();
    if b.get(1) != Some(&b'u') {
        return 2;
    }
    if b.get(2) == Some(&b'{') {
        return rest.find('}').map_or(rest.len(), |close| close + 1);
    }
    let one = 6.min(rest.len());
    // A JSON surrogate pair decodes as one character.
    let high = u32::from_str_radix(rest.get(2..6).unwrap_or(""), 16).ok();
    if high.is_some_and(|h| (0xD800..0xDC00).contains(&h)) && rest.get(6..8) == Some("\\u") {
        return 12.min(rest.len());
    }
    one
}
