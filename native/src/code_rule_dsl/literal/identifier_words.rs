//! ASCII identifier words in decoded text. This intentionally accepts arbitrary text, including
//! strings that are not valid type expressions. The word grammar is [A-Za-z_][A-Za-z0-9_]*.

use super::{Grammar, PNode};
use crate::code_rule_dsl::value::Value;

pub(crate) const KINDS: &[&str] = &["words", "word"];

pub(super) fn parse(text: &str) -> PNode {
    let mut root = PNode::new(Grammar::IdentifierWords, "words", 0, text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !c.is_ascii_alphabetic() && c != '_' {
            continue;
        }
        let mut end = start + 1;
        while let Some(&(at, next)) = chars.peek() {
            if !next.is_ascii_alphanumeric() && next != '_' {
                break;
            }
            end = at + 1;
            chars.next();
        }
        root.children.push(
            PNode::new(Grammar::IdentifierWords, "word", start, end)
                .with_value(Value::Str(text[start..end].to_owned())),
        );
    }
    root
}
