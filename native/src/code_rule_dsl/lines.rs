//! A file's lines as the linter a port follows reads them: where each line starts and ends.

use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::GrammarDialect;
use serde::{Deserialize, Serialize};

/// A diagnostic span: the byte offsets of its start and end, and their 0-based lines and byte
/// columns. It serializes as the JSON object of those six fields, its fields declared in key
/// order so a direct serialization writes the bytes the object's would.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Span {
    pub end_byte: usize,
    pub end_column: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub start_column: usize,
    pub start_line: usize,
}

/// The line terminators of a linter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Terminators {
    /// ESLint's `\r\n|[\r\n  ]` (`astUtils.createGlobalLinebreakMatcher`).
    Eslint,
    /// Ruff's `\r\n`, `\r` and `\n` (`ruff_source_file`) and PHP's tokenizer.
    Newlines,
}

impl Terminators {
    /// The linter a language's ports follow (`Adapter::TERMINATORS`).
    pub fn of(language: GrammarDialect) -> Self {
        crate::code_rule_dsl::language::base(Family::of(language)).terminators()
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Eslint => 0,
            Self::Newlines => 1,
        }
    }
}

/// Where each line of a file starts, and where its text ends before its terminator: u32s, `[n, starts[n], ends[n]]`
/// (decision 0353 step 4d). A loaded document's are placed (`placed`): in a slab's room beside its tape, where the
/// views' reads on the device read them (`rerail_clif::view_reads`'s `lines_line`). Others hold their own.
#[derive(Debug)]
pub struct LineTable {
    words: Words,
}

#[derive(Debug)]
enum Words {
    Own(Vec<u32>),
    Placed(std::sync::Arc<dyn rerail_tape_format::home::Room>),
}

/// Each terminator of `bytes` as `terminators` reads them, `(at, length)`, in order: a chunked search for their
/// first bytes (`memchr`), never a byte at a time.
fn terminators_of(bytes: &[u8], terminators: Terminators, mut each: impl FnMut(usize, usize)) {
    let mut skip = usize::MAX;
    let mut one = |at: usize| {
        if at == skip {
            return;
        }
        let length = match bytes[at] {
            b'\n' => 1,
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                skip = at + 1;
                2
            }
            b'\r' => 1,
            // U+2028 and U+2029: E2 80 A8 and E2 80 A9.
            0xE2 if bytes.get(at + 1) == Some(&0x80) && matches!(bytes.get(at + 2), Some(0xA8 | 0xA9)) => 3,
            _ => return,
        };
        each(at, length);
    };
    match terminators {
        Terminators::Eslint => memchr::memchr3_iter(b'\n', b'\r', 0xE2, bytes).for_each(&mut one),
        Terminators::Newlines => memchr::memchr2_iter(b'\n', b'\r', bytes).for_each(&mut one),
    }
}

impl LineTable {
    pub fn new(source: &str, terminators: Terminators) -> Self {
        let bytes = source.as_bytes();
        let mut n = 1;
        terminators_of(bytes, terminators, |_, _| n += 1);
        let mut words = vec![0u32; 1 + 2 * n];
        Self::fill(&mut words, bytes, terminators, n);
        Self { words: Words::Own(words) }
    }

    /// A loaded document's lines, in a slab's room where the device reads tapes (decision 0353 step 4d: the views'
    /// reads find them there), else its own.
    pub fn placed(source: &str, terminators: Terminators) -> Self {
        let bytes = source.as_bytes();
        let mut n = 1;
        terminators_of(bytes, terminators, |_, _| n += 1);
        let len = 4 * (1 + 2 * n);
        let home = crate::code_rule_dsl::tape::placed::place(len, |out| {
            // SAFETY: the room is 16-byte aligned (`tape_room`), `len` bytes of it this fill's alone.
            let (pre, words, _) = unsafe { out.align_to_mut::<u32>() };
            assert!(pre.is_empty(), "a room is aligned");
            Self::fill(words, bytes, terminators, n);
            Ok(())
        });
        match home {
            Ok(rerail_tape_format::home::Home::Placed(room)) => Self { words: Words::Placed(room) },
            _ => Self::new(source, terminators),
        }
    }

    /// `words` (`1 + 2 * n`): the count, the starts, the ends.
    fn fill(words: &mut [u32], bytes: &[u8], terminators: Terminators, n: usize) {
        let word = |at: usize| u32::try_from(at).expect("a file within 4 GB");
        words[0] = word(n);
        words[1] = 0;
        let mut line = 0;
        terminators_of(bytes, terminators, |at, length| {
            words[1 + n + line] = word(at);
            line += 1;
            words[1 + line] = word(at + length);
        });
        words[1 + n + line] = word(bytes.len());
    }

    /// The words: `[n, starts[n], ends[n]]`.
    pub fn words(&self) -> &[u32] {
        match &self.words {
            Words::Own(w) => w,
            Words::Placed(room) => {
                // SAFETY: the room holds the words `placed` wrote, at its 16-byte aligned start.
                let (_, words, _) = unsafe { room.bytes().align_to::<u32>() };
                words
            }
        }
    }

    /// The room the words are in, if they are placed.
    pub fn room(&self) -> Option<&std::sync::Arc<dyn rerail_tape_format::home::Room>> {
        match &self.words {
            Words::Own(_) => None,
            Words::Placed(room) => Some(room),
        }
    }

    fn n(&self) -> usize {
        self.words()[0] as usize
    }

    /// Tree-sitter's rows (`TSPoint`): only `\n` ends one. A lone `\r`, U+2028 and U+2029 stay
    /// inside a row, where the linters' lines end at them.
    pub(crate) fn tree_sitter(source: &str) -> Self {
        let bytes = source.as_bytes();
        let n = 1 + memchr::memchr_iter(b'\n', bytes).count();
        let mut words = vec![0u32; 1 + 2 * n];
        words[0] = n as u32;
        for (line, at) in memchr::memchr_iter(b'\n', bytes).enumerate() {
            words[1 + n + line] = at as u32;
            words[2 + line] = at as u32 + 1;
        }
        words[2 * n] = bytes.len() as u32;
        Self { words: Words::Own(words) }
    }

    /// Byte offset of each line's first byte.
    pub fn starts(&self) -> &[u32] {
        let n = self.n();
        &self.words()[1..1 + n]
    }

    fn ends(&self) -> &[u32] {
        let n = self.n();
        &self.words()[1 + n..1 + 2 * n]
    }

    /// The 0-based line holding `byte`; a terminator's bytes belong to the line they end.
    pub fn line(&self, byte: usize) -> usize {
        self.starts().partition_point(|&start| start as usize <= byte) - 1
    }

    /// The 0-based line and byte column holding `byte`.
    pub fn point(&self, byte: usize) -> (usize, usize) {
        let line = self.line(byte);
        (line, byte - self.start(line))
    }

    /// The diagnostic span at two byte offsets.
    pub fn span(&self, start: usize, end: usize) -> Span {
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
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + 4 * self.words().len()
    }

    /// The first byte of `line`.
    pub fn start(&self, line: usize) -> usize {
        self.starts()[line] as usize
    }

    /// Where `line`'s text ends: its terminator's first byte, or the file's end.
    pub fn end(&self, line: usize) -> usize {
        self.ends()[line] as usize
    }

    /// Where the line after `line` starts, or the file's end on the last line (Ruff's
    /// `full_line_end`).
    pub fn full_end(&self, line: usize) -> usize {
        self.starts()
            .get(line + 1)
            .map_or_else(|| self.end(line), |&s| s as usize)
    }

    /// The number of lines: one more than the terminators.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn len(&self) -> usize {
        self.n()
    }
}

// cratify: `eslint` is rerail-classic's (it mounts it)

#[cfg(test)]
pub mod tests {
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
