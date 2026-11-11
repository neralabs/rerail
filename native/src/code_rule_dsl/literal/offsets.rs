//! A Python string literal's decoded text with the source span of every decoded character, so a
//! node parsed from the text maps back to the bytes it came from.

use std::ops::Range;

/// One decoded character: its byte offset in the decoded text and the source bytes it came from.
/// An escape maps to the whole escape, `\r\n` to both bytes, an f-string's `{{` to both braces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub at: u32,
    pub start: u32,
    pub end: u32,
}

/// Ruff's "simple" string (`parse_type_annotation`): one part whose raw content equals its value,
/// so offsets into the value are offsets into the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Simple {
    /// Where the content starts in the source, after the prefix and the opening quotes.
    pub content: usize,
    pub triple: bool,
}

#[derive(Clone, Debug)]
pub struct Decoded {
    pub text: String,
    pub spans: Vec<Span>,
    /// The literal node's source range.
    pub host: Range<usize>,
    pub simple: Option<Simple>,
}

impl Decoded {
    /// The source range of the decoded bytes `range`: the first character's span start to the
    /// last one's end; an empty range sits at the next character's start (or after the last).
    pub fn host_range(&self, range: Range<usize>) -> Range<usize> {
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

pub enum Decode {
    Known(Decoded),
    /// A string whose value is not known: an f-string with a field, a `t` string, bytes, a value
    /// past 64 KiB, an escape Python refuses.
    Unknown,
    /// Not a string literal.
    NotString,
}


pub fn push(decoded: &mut Decoded, c: char, start: usize, end: usize) {
    decoded.spans.push(Span {
        at: decoded.text.len() as u32,
        start: start as u32,
        end: end as u32,
    });
    decoded.text.push(c);
}



#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::literal::python_string::decode_python;
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
