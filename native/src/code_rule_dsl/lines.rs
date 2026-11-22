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

/// The line terminators of a language's ecosystem: its compiler's, its linters' and its editors' (decision 0612), the one
/// definition of a line (`LineTable`) and of every place on it. A `\n\r` is two of them wherever `\r` is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Terminators {
    /// ESLint's `\r\n|[\r\n  ]` (`astUtils.createGlobalLinebreakMatcher`), TypeScript's: ECMAScript's line
    /// terminators. And a file of no language's.
    Eslint,
    /// `\r\n`, `\r` and `\n`: CPython's tokenizer and Ruff's (`ruff_source_file`), the JVM languages' lexers, PHP's,
    /// clang's, Swift's, HTML's and CommonMark's, and the Language Server Protocol's.
    Newlines,
    /// `\n` alone, a `\r` before it left out of the line's text: rustc's (`SourceFile::lines`) and Go's (`go/token`),
    /// which their linters report on. A lone `\r` stays inside a line.
    Lf,
    /// Roslyn's (`SyntaxFacts::IsNewLine`): `\r\n`, `\r`, `\n`, U+0085, U+2028 and U+2029.
    Roslyn,
    /// CSS Syntax's (§3.3, its preprocessing): `\r\n`, `\r`, `\n` and a form feed.
    Css,
}

impl Terminators {
    /// The linter a language's ports follow (`Adapter::TERMINATORS`).
    pub fn of(language: GrammarDialect) -> Self {
        crate::code_rule_dsl::language::base(Family::of(language)).terminators()
    }

    /// The sets' number: `COUNT` of them.
    pub const fn index(self) -> usize {
        match self {
            Self::Eslint => 0,
            Self::Newlines => 1,
            Self::Lf => 2,
            Self::Roslyn => 3,
            Self::Css => 4,
        }
    }
    pub const COUNT: usize = 5;
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
            // `\n` alone ends a line there: a `\r` before it is the terminator's, out of the line's text.
            b'\n' if terminators == Terminators::Lf && at > 0 && bytes[at - 1] == b'\r' => {
                each(at - 1, 2);
                return;
            }
            b'\n' => 1,
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                skip = at + 1;
                2
            }
            b'\r' => 1,
            0x0C => 1,
            // U+0085: C2 85.
            0xC2 if bytes.get(at + 1) == Some(&0x85) => 2,
            // U+2028 and U+2029: E2 80 A8 and E2 80 A9.
            0xE2 if bytes.get(at + 1) == Some(&0x80) && matches!(bytes.get(at + 2), Some(0xA8 | 0xA9)) => 3,
            _ => return,
        };
        each(at, length);
    };
    match terminators {
        Terminators::Eslint => memchr::memchr3_iter(b'\n', b'\r', 0xE2, bytes).for_each(&mut one),
        Terminators::Newlines => memchr::memchr2_iter(b'\n', b'\r', bytes).for_each(&mut one),
        Terminators::Lf => memchr::memchr_iter(b'\n', bytes).for_each(&mut one),
        Terminators::Css => memchr::memchr3_iter(b'\n', b'\r', 0x0C, bytes).for_each(&mut one),
        // Four first bytes: two searches, merged in order.
        Terminators::Roslyn => {
            let mut wide = memchr::memchr_iter(0xC2, bytes).peekable();
            for at in memchr::memchr3_iter(b'\n', b'\r', 0xE2, bytes) {
                while let Some(c) = wide.next_if(|&c| c < at) {
                    one(c);
                }
                one(at);
            }
            wide.for_each(&mut one);
        }
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

    /// The number of lines: one more than the terminators, the empty line after a final one included (core.rerail's
    /// `src_line`).
    pub fn count(&self) -> usize {
        self.n()
    }

    /// The lines a linter counts in the file (core.rerail's `line_count`: ESLint's `max-lines`, pylint's
    /// `too-many-lines`): the empty line after a final terminator left out, so an empty file has none.
    pub fn counted(&self) -> usize {
        let n = self.n();
        n - usize::from(self.start(n - 1) == self.end(n - 1))
    }

    /// Line `l`'s text, `[start, end)`, from 1: its first byte and its terminator's.
    pub fn text_of(&self, l: usize) -> [u32; 2] {
        [self.starts()[l - 1], self.ends()[l - 1]]
    }

    /// A span's place, as core.rerail's `line`, `end_line` and `col` give it (one definition for every node, token
    /// and span): the line of its start, the line holding its end offset, both from 1, and its start's byte column.
    /// A terminator's bytes are the line's they end, so a span ending with one ends on the line after it.
    pub fn place(&self, [start, end]: [u32; 2]) -> (u32, u32, u32) {
        let line = self.line(start as usize);
        let last = self.line(end as usize);
        (line as u32 + 1, last as u32 + 1, start - self.starts()[line])
    }
}

// cratify: `eslint` is rerail-workspace's (it mounts it)

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

    /// Each ecosystem's line ends (decision 0612): rustc's and Go's `\n` alone (a `\r` before one out of the line's
    /// text, a lone one inside it), Roslyn's U+0085, U+2028 and U+2029 beside `\r\n`, `\r` and `\n`, CSS's form feed;
    /// a `\n\r` is two wherever `\r` ends a line.
    #[test]
    fn lines_end_at_each_ecosystem_s_terminators() {
        let source = "a\r\nb\rc\n\rd\u{85}e\u{2028}f\x0cg\n";
        let texts = |terminators: Terminators| {
            let table = LineTable::new(source, terminators);
            (1..=table.count()).map(|l| table.text_of(l)).map(|[a, b]| &source[a as usize..b as usize]).collect::<Vec<_>>()
        };
        assert_eq!(texts(Terminators::Lf), ["a", "b\rc", "\rd\u{85}e\u{2028}f\x0cg", ""]);
        assert_eq!(texts(Terminators::Newlines), ["a", "b", "c", "", "d\u{85}e\u{2028}f\x0cg", ""]);
        assert_eq!(texts(Terminators::Eslint), ["a", "b", "c", "", "d\u{85}e", "f\x0cg", ""]);
        assert_eq!(texts(Terminators::Roslyn), ["a", "b", "c", "", "d", "e", "f\x0cg", ""]);
        assert_eq!(texts(Terminators::Css), ["a", "b", "c", "", "d\u{85}e\u{2028}f", "g", ""]);
    }

    /// A place, as `line`, `end_line` and `col` give it: the line of a span's start, the line holding its end offset
    /// (a span ending with a terminator ends on the line after it, as ESLint's and Ruff's end locations do), its
    /// start's byte column; the lines a linter counts leave out the empty line after a final terminator.
    #[test]
    fn a_place_is_read_off_the_lines() {
        let t = LineTable::new("ab\r\ncd\n", Terminators::Newlines);
        assert_eq!(t.place([0, 2]), (1, 1, 0));
        assert_eq!(t.place([0, 4]), (1, 2, 0));
        assert_eq!(t.place([5, 6]), (2, 2, 1));
        assert_eq!(t.place([0, 7]), (1, 3, 0));
        assert_eq!(t.place([7, 7]), (3, 3, 0));
        assert_eq!((t.count(), t.counted()), (3, 2));
        assert_eq!((LineTable::new("", Terminators::Lf).count(), LineTable::new("", Terminators::Lf).counted()), (1, 0));
        assert_eq!(LineTable::new("x", Terminators::Lf).counted(), 1);
    }
}
