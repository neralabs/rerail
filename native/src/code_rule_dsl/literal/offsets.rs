//! A Python string literal's decoded text with the source span of every decoded character, so a
//! node parsed from the text maps back to the bytes it came from.

use crate::code_rule_dsl::value::MAX_STRING_BYTES;
use crate::code_rule_dsl::value::names;
use crate::code_rule_dsl::value::python::{Prefix, prefix};
use ast_grep_core::{Doc, Node};
use std::ops::Range;

/// One decoded character: its byte offset in the decoded text and the source bytes it came from.
/// An escape maps to the whole escape, `\r\n` to both bytes, an f-string's `{{` to both braces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    pub(crate) at: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// Ruff's "simple" string (`parse_type_annotation`): one part whose raw content equals its value,
/// so offsets into the value are offsets into the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Simple {
    /// Where the content starts in the source, after the prefix and the opening quotes.
    pub(crate) content: usize,
    pub(crate) triple: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Decoded {
    pub(crate) text: String,
    pub(crate) spans: Vec<Span>,
    /// The literal node's source range.
    pub(crate) host: Range<usize>,
    pub(crate) simple: Option<Simple>,
}

impl Decoded {
    /// The source range of the decoded bytes `range`: the first character's span start to the
    /// last one's end; an empty range sits at the next character's start (or after the last).
    pub(crate) fn host_range(&self, range: Range<usize>) -> Range<usize> {
        let index = |at: usize| self.spans.partition_point(|span| (span.at as usize) < at);
        let first = index(range.start);
        let start = match self.spans.get(first) {
            Some(span) => span.start as usize,
            None => self
                .spans
                .last()
                .map_or(self.host.start, |span| span.end as usize),
        };
        if range.end <= range.start {
            return start..start;
        }
        let last = index(range.end).saturating_sub(1).max(first);
        let end = self.spans.get(last).map_or(start, |span| span.end as usize);
        start..end.max(start)
    }
}

pub(crate) enum Decode {
    Known(Decoded),
    /// A string whose value is not known: an f-string with a field, a `t` string, bytes, a value
    /// past 64 KiB, an escape Python refuses.
    Unknown,
    /// Not a string literal.
    NotString,
}

/// Decodes a Python `string` or `concatenated_string` node as `str`, recording spans.
pub(crate) fn decode_python<D: Doc>(node: &Node<'_, D>) -> Decode {
    let parts: Vec<Node<'_, D>> = match node.kind().as_ref() {
        "string" => vec![node.clone()],
        "concatenated_string" => node
            .children()
            .filter(|child| child.kind() == "string")
            .collect(),
        _ => return Decode::NotString,
    };
    let mut decoded = Decoded {
        text: String::new(),
        spans: Vec::new(),
        host: node.range(),
        simple: None,
    };
    let single = parts.len() == 1;
    for part in &parts {
        if part
            .children()
            .any(|child| matches!(child.kind().as_ref(), "interpolation"))
        {
            return Decode::Unknown;
        }
        let text = part.text();
        let base = part.range().start;
        let Some(quote_at) = text.find(['\'', '"']) else {
            return Decode::Unknown;
        };
        let Some(prefix) = prefix(&text[..quote_at]) else {
            return Decode::Unknown;
        };
        if prefix.template || prefix.bytes {
            return Decode::Unknown;
        }
        let rest = &text[quote_at..];
        let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") {
            3
        } else {
            1
        };
        if rest.len() < 2 * quote || !rest.ends_with(&rest[..quote]) {
            return Decode::Unknown;
        }
        let body = &rest[quote..rest.len() - quote];
        let body_start = base + quote_at + quote;
        let before = decoded.text.len();
        if !decode_body(body, body_start, &prefix, &mut decoded) {
            return Decode::Unknown;
        }
        if decoded.text.len() > MAX_STRING_BYTES {
            return Decode::Unknown;
        }
        if single && !prefix.format && decoded.text[before..] == *body {
            decoded.simple = Some(Simple {
                content: body_start,
                triple: quote == 3,
            });
        }
    }
    Decode::Known(decoded)
}

pub(super) fn push(decoded: &mut Decoded, c: char, start: usize, end: usize) {
    decoded.spans.push(Span {
        at: decoded.text.len() as u32,
        start: start as u32,
        end: end as u32,
    });
    decoded.text.push(c);
}

/// Decodes one literal's body into `decoded`: universal newlines, an f-string's doubled braces,
/// escapes unless raw. False for an escape Python refuses or a name `\N{...}` does not know.
pub(super) fn decode_body(body: &str, base: usize, prefix: &Prefix, decoded: &mut Decoded) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < body.len() {
        let c = body[index..].chars().next().expect("in bounds");
        let at = base + index;
        match c {
            '\r' => {
                let width = if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                push(decoded, '\n', at, at + width);
                index += width;
            }
            '{' | '}' if prefix.format && bytes.get(index + 1) == Some(&(c as u8)) => {
                push(decoded, c, at, at + 2);
                index += 2;
            }
            '\\' if !prefix.raw => {
                let Some(next) = escape(body, index, at, decoded) else {
                    return false;
                };
                index = next;
            }
            _ => {
                push(decoded, c, at, at + c.len_utf8());
                index += c.len_utf8();
            }
        }
    }
    true
}

/// Decodes the escape at `index` (a backslash), returning the index after it.
fn escape(body: &str, index: usize, at: usize, decoded: &mut Decoded) -> Option<usize> {
    let rest = &body[index + 1..];
    let escape = rest.chars().next()?;
    let after = index + 1 + escape.len_utf8();
    let whole = |end: usize| at..at + (end - index);
    let simple = match escape {
        '\n' => return Some(after),
        '\r' => {
            return Some(if rest.as_bytes().get(1) == Some(&b'\n') {
                after + 1
            } else {
                after
            });
        }
        '\\' => Some('\\'),
        '\'' => Some('\''),
        '"' => Some('"'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'v' => Some('\u{b}'),
        _ => None,
    };
    if let Some(c) = simple {
        let range = whole(after);
        push(decoded, c, range.start, range.end);
        return Some(after);
    }
    let hex = |count: usize| -> Option<(u32, usize)> {
        let digits = rest.get(1..1 + count)?;
        (digits.len() == count && digits.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(digits, 16).ok())
            .flatten()
            .map(|code| (code, after + count))
    };
    let (code, end) = match escape {
        '0'..='7' => {
            let mut code = escape.to_digit(8)?;
            let mut end = after;
            for digit in rest[1..].chars().take(2) {
                match digit.to_digit(8) {
                    Some(digit) => {
                        code = code * 8 + digit;
                        end += 1;
                    }
                    None => break,
                }
            }
            (code, end)
        }
        'x' => hex(2)?,
        'u' => hex(4)?,
        'U' => hex(8)?,
        'N' => {
            let name = rest.strip_prefix("N{")?;
            let close = name.find('}')?;
            let c = names::lookup(&name[..close])?;
            (u32::from(c), after + 1 + close + 1)
        }
        other => {
            push(decoded, '\\', at, at + 1);
            push(decoded, other, at + 1, at + 1 + other.len_utf8());
            return Some(after);
        }
    };
    let c = char::from_u32(code)?;
    let range = whole(end);
    push(decoded, c, range.start, range.end);
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;

    fn decode(source: &str) -> Option<Decoded> {
        let root = GrammarDialect::Python.ast_grep(source);
        let node = root
            .root()
            .dfs()
            .find(|node| matches!(node.kind().as_ref(), "string" | "concatenated_string"))
            .expect("a string");
        match decode_python(&node) {
            Decode::Known(decoded) => Some(decoded),
            _ => None,
        }
    }

    /// Each decoded character with the source text it came from.
    fn pieces(source: &str) -> Vec<(char, String)> {
        let decoded = decode(source).expect("decodes");
        decoded
            .text
            .chars()
            .zip(&decoded.spans)
            .map(|(c, span)| (c, source[span.start as usize..span.end as usize].to_owned()))
            .collect()
    }

    #[test]
    fn escapes_map_to_the_whole_escape() {
        assert_eq!(
            pieces(r#"x = "a\x41\n\N{BULLET}é\101\q""#),
            [
                ('a', "a".to_owned()),
                ('A', r"\x41".to_owned()),
                ('\n', r"\n".to_owned()),
                ('•', r"\N{BULLET}".to_owned()),
                ('é', r"é".to_owned()),
                ('A', r"\101".to_owned()),
                ('\\', r"\".to_owned()),
                ('q', "q".to_owned()),
            ]
        );
        // A line continuation decodes to nothing; CRLF is one newline.
        assert_eq!(
            pieces("x = '''a\\\nb\r\nc'''"),
            [
                ('a', "a".to_owned()),
                ('b', "b".to_owned()),
                ('\n', "\r\n".to_owned()),
                ('c', "c".to_owned()),
            ]
        );
    }

    #[test]
    fn implicit_concatenation_chains_the_parts() {
        let source = "x = ('a' r'\\n' \"b\")";
        assert_eq!(
            pieces(source),
            [
                ('a', "a".to_owned()),
                ('\\', "\\".to_owned()),
                ('n', "n".to_owned()),
                ('b', "b".to_owned()),
            ]
        );
        let decoded = decode(source).expect("decodes");
        assert_eq!(decoded.simple, None);
        // A range maps from its first character's start to its last one's end.
        let range = decoded.host_range(1..3);
        assert_eq!(&source[range], "\\n");
        assert_eq!(&source[decoded.host_range(4..4)], "");
        assert_eq!(decoded.host_range(4..4).start, source.len() - 2);
    }

    #[test]
    fn simple_strings_are_single_parts_whose_source_is_their_value() {
        let decoded = decode("x: 'List[int]'").expect("decodes");
        assert_eq!(
            decoded.simple,
            Some(Simple {
                content: 4,
                triple: false
            })
        );
        assert_eq!(
            decode("x: \"\"\"int\"\"\"")
                .expect("decodes")
                .simple
                .map(|s| s.triple),
            Some(true)
        );
        assert_eq!(decode(r#"x: "a\tb""#).expect("decodes").simple, None);
        assert_eq!(
            decode(r#"x: r"a\tb""#)
                .expect("decodes")
                .simple
                .map(|s| s.content),
            Some(5)
        );
    }

    #[test]
    fn unknown_strings() {
        assert!(decode("x = f'{a}'").is_none());
        assert!(decode("x = b'a'").is_none());
        assert!(decode(r"x = '\N{NO SUCH NAME}'").is_none());
        assert!(decode(r"x = '\x4'").is_none());
        // An f-string without fields decodes its doubled braces.
        assert_eq!(
            pieces("x = f'{{a}}'"),
            [
                ('{', "{{".to_owned()),
                ('a', "a".to_owned()),
                ('}', "}}".to_owned()),
            ]
        );
    }
}
