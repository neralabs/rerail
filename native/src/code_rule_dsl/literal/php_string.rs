//! A PHP string literal's value as text, mapped back to the source: the value is the one
//! `value::php` gives the literal (PHP 8.5's escapes, `b` prefixes, heredoc indentation), and each
//! decoded character keeps the source bytes it came from. A quoted string maps piece by piece
//! (tree-sitter-php's `string_content` byte for byte, an `escape_sequence` to the whole escape); a
//! heredoc or nowdoc, whose indentation PHP strips, maps every node to the whole literal.

use super::HostMap;
use super::offsets::{Decoded, Span};
use crate::code_rule_dsl::value::{Context, MAX_STRING_BYTES, Value, php};
use ast_grep_core::{Doc, Node};
use std::ops::Range;

/// What a PHP literal decodes to.
pub(crate) enum PhpLiteral {
    /// Its value as text, and how the text maps to the source.
    Known(String, HostMap),
    /// A string whose value is not known: one that interpolates, an escape PHP refuses, bytes
    /// that are not UTF-8, a value past 64 KiB.
    Unknown,
    /// Not a string literal.
    NotString,
}

/// Decodes a PHP `string`, `encapsed_string`, `heredoc` or `nowdoc` node.
pub(crate) fn decode<D: Doc>(node: &Node<'_, D>) -> PhpLiteral {
    let kind = node.kind();
    if !matches!(
        kind.as_ref(),
        "string" | "encapsed_string" | "heredoc" | "nowdoc"
    ) {
        return PhpLiteral::NotString;
    }
    let mut spend = || true;
    let Value::PhpStr(bytes) = php::decode(node, &mut Context::new(&mut spend)) else {
        return PhpLiteral::Unknown;
    };
    let Some(text) = text(bytes) else {
        return PhpLiteral::Unknown;
    };
    let map = match kind.as_ref() {
        "string" | "encapsed_string" => spans(node, &text, kind == "encapsed_string")
            .map_or_else(|| HostMap::Whole(node.range()), HostMap::Spans),
        _ => HostMap::Whole(node.range()),
    };
    PhpLiteral::Known(text, map)
}

/// A PHP string's bytes as text: `None` for bytes that are not UTF-8 or past 64 KiB.
pub(crate) fn text(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > MAX_STRING_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The per-character spans of a quoted literal, from its pieces; `None` where the pieces do not
/// decode to `text` piece by piece (the map is then the whole literal).
fn spans<D: Doc>(node: &Node<'_, D>, text: &str, double: bool) -> Option<Decoded> {
    // The source range of every decoded byte.
    let mut bytes: Vec<u8> = Vec::with_capacity(text.len());
    let mut sources: Vec<Range<usize>> = Vec::with_capacity(text.len());
    for piece in node.children().filter(|child| child.is_named()) {
        let raw = piece.text();
        let range = piece.range();
        let decoded = match piece.kind().as_ref() {
            "string_content" if !double => raw.as_bytes().to_vec(),
            "escape_sequence" if !double => raw.as_bytes()[1..].to_vec(),
            "string_content" | "escape_sequence" => php::unescape(raw.as_bytes(), true)?,
            _ => return None,
        };
        if decoded == raw.as_bytes() {
            sources.extend((range.start..range.end).map(|at| at..at + 1));
        } else {
            sources.extend(std::iter::repeat_n(range.clone(), decoded.len()));
        }
        bytes.extend(decoded);
    }
    if bytes != text.as_bytes() {
        return None;
    }
    let spans = text
        .char_indices()
        .map(|(at, c)| {
            let source = &sources[at..at + c.len_utf8()];
            Span {
                at: at as u32,
                start: source.iter().map(|range| range.start).min().unwrap_or(0) as u32,
                end: source.iter().map(|range| range.end).max().unwrap_or(0) as u32,
            }
        })
        .collect();
    Some(Decoded {
        text: text.to_owned(),
        spans,
        host: node.range(),
        simple: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;

    /// Each decoded character of the first string literal in `source`, with its source text.
    fn mapped(source: &str) -> (String, Vec<(char, &str)>) {
        let root = GrammarDialect::Php.ast_grep(source);
        let node = root
            .root()
            .dfs()
            .find(|node| {
                matches!(
                    node.kind().as_ref(),
                    "string" | "encapsed_string" | "heredoc" | "nowdoc"
                )
            })
            .expect("a string");
        let PhpLiteral::Known(text, map) = decode(&node) else {
            panic!("known")
        };
        let chars = text
            .char_indices()
            .map(|(at, c)| {
                let range = match &map {
                    HostMap::Spans(decoded) => decoded.host_range(at..at + c.len_utf8()),
                    HostMap::Whole(range) => range.clone(),
                    HostMap::Exact(_) => unreachable!(),
                };
                (c, &source[range])
            })
            .collect();
        (text, chars)
    }

    #[test]
    fn escapes_map_to_their_source() {
        let (text, chars) = mapped("<?php f('a\\'b\\\\c\\n');");
        assert_eq!(text, "a'b\\c\\n");
        assert_eq!(
            chars,
            [
                ('a', "a"),
                ('\'', "\\'"),
                ('b', "b"),
                ('\\', "\\\\"),
                ('c', "c"),
                ('\\', "\\"),
                ('n', "n")
            ]
        );
        let (text, chars) = mapped("<?php f(\"%\\x73 \\u{e9}\");");
        assert_eq!(text, "%s é");
        assert_eq!(
            chars,
            [('%', "%"), ('s', "\\x73"), (' ', " "), ('é', "\\u{e9}")]
        );
    }

    #[test]
    fn heredocs_map_to_the_whole_literal() {
        let source = "<?php\nf(<<<EOT\n    %s\n      %d\n    EOT);\n";
        let (text, chars) = mapped(source);
        assert_eq!(text, "%s\n  %d");
        assert!(chars.iter().all(|(_, at)| at.starts_with("<<<EOT")));
    }

    #[test]
    fn interpolation_and_bytes_are_unknown() {
        for source in ["<?php f(\"%s $x\");", "<?php f(\"\\xff\");"] {
            let root = GrammarDialect::Php.ast_grep(source);
            let node = root
                .root()
                .dfs()
                .find(|node| node.kind() == "encapsed_string")
                .expect("a string");
            assert!(matches!(decode(&node), PhpLiteral::Unknown), "{source}");
        }
    }
}
