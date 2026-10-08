//! A file's lines as the linter a port follows reads them: where each line starts and ends.

use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::GrammarDialect;
use serde::{Deserialize, Serialize};

/// A diagnostic span: the byte offsets of its start and end, and their 0-based lines and byte
/// columns. It serializes as the JSON object of those six fields, its fields declared in key
/// order so a direct serialization writes the bytes the object's would.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Span {
    pub(crate) end_byte: usize,
    pub(crate) end_column: usize,
    pub(crate) end_line: usize,
    pub(crate) start_byte: usize,
    pub(crate) start_column: usize,
    pub(crate) start_line: usize,
}

/// The line terminators of a linter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Terminators {
    /// ESLint's `\r\n|[\r\n  ]` (`astUtils.createGlobalLinebreakMatcher`).
    Eslint,
    /// Ruff's `\r\n`, `\r` and `\n` (`ruff_source_file`) and PHP's tokenizer.
    Newlines,
}

impl Terminators {
    /// The linter a language's ports follow.
    pub(crate) fn of(language: GrammarDialect) -> Self {
        match Family::of(language) {
            Family::Python | Family::Php | Family::Blade => Self::Newlines,
            Family::Ecma => Self::Eslint,
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Eslint => 0,
            Self::Newlines => 1,
        }
    }
}

/// Where each line of a file starts, and where its text ends before its terminator.
#[derive(Debug)]
pub(crate) struct LineTable {
    /// Byte offset of each line's first byte; the first is 0.
    starts: Vec<usize>,
    /// Byte offset where each line's terminator starts, or the file's end on the last line.
    ends: Vec<usize>,
}

impl LineTable {
    pub(crate) fn new(source: &str, terminators: Terminators) -> Self {
        let bytes = source.as_bytes();
        let mut starts = vec![0];
        let mut ends = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let length = match bytes[at] {
                b'\n' => 1,
                b'\r' if bytes.get(at + 1) == Some(&b'\n') => 2,
                b'\r' => 1,
                // U+2028 and U+2029: E2 80 A8 and E2 80 A9.
                0xE2 if terminators == Terminators::Eslint
                    && bytes.get(at + 1) == Some(&0x80)
                    && matches!(bytes.get(at + 2), Some(0xA8 | 0xA9)) =>
                {
                    3
                }
                _ => {
                    at += 1;
                    continue;
                }
            };
            ends.push(at);
            at += length;
            starts.push(at);
        }
        ends.push(bytes.len());
        Self { starts, ends }
    }

    /// Tree-sitter's rows (`TSPoint`): only `\n` ends one. A lone `\r`, U+2028 and U+2029 stay
    /// inside a row, where the linters' lines end at them.
    pub(crate) fn tree_sitter(source: &str) -> Self {
        let mut starts = vec![0];
        let mut ends = Vec::new();
        for (at, _) in source
            .bytes()
            .enumerate()
            .filter(|&(_, byte)| byte == b'\n')
        {
            ends.push(at);
            starts.push(at + 1);
        }
        ends.push(source.len());
        Self { starts, ends }
    }

    /// The 0-based line holding `byte`; a terminator's bytes belong to the line they end.
    pub(crate) fn line(&self, byte: usize) -> usize {
        self.starts.partition_point(|start| *start <= byte) - 1
    }

    /// The 0-based line and byte column holding `byte`.
    pub(crate) fn point(&self, byte: usize) -> (usize, usize) {
        let line = self.line(byte);
        (line, byte - self.start(line))
    }

    /// The diagnostic span at two byte offsets.
    pub(crate) fn span(&self, start: usize, end: usize) -> Span {
        let (start_line, start_column) = self.point(start);
        let (end_line, end_column) = self.point(end);
        Span {
            end_byte: end,
            end_column,
            end_line,
            start_byte: start,
            start_column,
            start_line,
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(crate) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.starts.capacity() * std::mem::size_of::<usize>()
            + self.ends.capacity() * std::mem::size_of::<usize>()
    }

    /// The first byte of `line`.
    pub(crate) fn start(&self, line: usize) -> usize {
        self.starts[line]
    }

    /// Where `line`'s text ends: its terminator's first byte, or the file's end.
    pub(crate) fn end(&self, line: usize) -> usize {
        self.ends[line]
    }

    /// Where the line after `line` starts, or the file's end on the last line (Ruff's
    /// `full_line_end`).
    pub(crate) fn full_end(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .copied()
            .unwrap_or_else(|| self.ends[line])
    }

    /// The number of lines: one more than the terminators.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }
}

#[cfg(test)]
mod eslint;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_end_at_each_linter_s_terminators() {
        let source = "a\r\nb\rc\nd\u{2028}e\u{2029}f";
        let eslint = LineTable::new(source, Terminators::Eslint);
        let python = LineTable::new(source, Terminators::Newlines);
        assert_eq!(eslint.len(), 6);
        assert_eq!(python.len(), 4);
        let texts = |table: &LineTable| {
            (0..table.len())
                .map(|line| &source[table.start(line)..table.end(line)])
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(&eslint), ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(texts(&python), ["a", "b", "c", "d\u{2028}e\u{2029}f"]);
        // A terminator belongs to the line it ends.
        assert_eq!(eslint.line(0), 0);
        assert_eq!(eslint.line(1), 0);
        assert_eq!(eslint.line(2), 0);
        assert_eq!(eslint.line(3), 1);
        assert_eq!(eslint.line(source.len()), 5);
        assert_eq!(python.line(source.len()), 3);
        assert_eq!(eslint.full_end(0), 3);
        assert_eq!(eslint.full_end(5), source.len());
        let empty = LineTable::new("", Terminators::Newlines);
        assert_eq!((empty.len(), empty.line(0), empty.end(0)), (1, 0, 0));
        let trailing = LineTable::new("x\n", Terminators::Newlines);
        assert_eq!(
            (trailing.len(), trailing.line(2), trailing.end(1)),
            (2, 1, 2)
        );
    }
}
