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

/// The line terminators of a language's ecosystem, the tape's (`rerail_tape_format::newlines`, decision 0612).
pub use rerail_tape_format::newlines::Terminators;

/// The terminators a language's ecosystem ends a line at (`Adapter::TERMINATORS`, decision 0612).
pub fn terminators(language: GrammarDialect) -> Terminators {
    crate::code_rule_dsl::language::base(Family::of(language)).terminators()
}

/// `terminators`, of a file that may have no language: one with none ends a line at ECMAScript's terminators (W's,
/// decision 0612's table).
pub fn terminators_of(language: Option<GrammarDialect>) -> Terminators {
    language.map_or(Terminators::Eslint, terminators)
}

/// The line (from 1) of byte `byte` of the file at `path` (its language by its path, `terminators_of`): an oracle's
/// line of a violation.
pub fn line_at_path(path: &str, source: &str, byte: usize) -> usize {
    let language = crate::code_rule_dsl::language::label_of_path(path).and_then(crate::code_rule_dsl::language::dialect_by_label);
    LineTable::new(source, terminators_of(language)).line(byte) + 1
}

/// The line (from 1) of byte `byte` of a `language` file, by its lines (an oracle's line, a message's).
pub fn line_of(source: &str, language: GrammarDialect, byte: usize) -> usize {
    LineTable::new(source, terminators(language)).line(byte) + 1
}

/// A file's lines, read off its line index (`rerail_tape_format::newlines`, decision 0612): its tape's, where it has
/// a format 16 tape (`of_tape`, shared with it), else one of its own (`new`), placed in a slab's room where the
/// device reads tapes (`placed`). The index holds each terminator's last byte; a line's start is one past the one
/// before it, and its text's end is that terminator's first byte, from the source's bytes (`end`). One read of a
/// line, the tape's (`rerail_tape_format::reads::f16::line`), on the host, in the JIT and on the device.
#[derive(Debug)]
pub struct LineTable {
    home: Index,
    /// Where the index starts in `home`.
    at: u32,
    /// The source's length.
    len: u32,
}

/// Where a document's line index is: its own (or placed), or its format 16 tape's.
#[derive(Debug)]
enum Index {
    Own(rerail_tape_format::home::Home),
    F16(std::sync::Arc<rerail_tape_format::f16::Tape16>),
    F16Z(std::sync::Arc<rerail_tape_format::f16::Tape16Z>),
}

impl std::ops::Deref for Index {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Index::Own(h) => h,
            Index::F16(t) => t.extent(),
            Index::F16Z(t) => t.extent(),
        }
    }
}

impl Index {
    fn home(&self) -> &rerail_tape_format::home::Home {
        match self {
            Index::Own(h) => h,
            Index::F16(t) => t.home(),
            Index::F16Z(t) => t.home(),
        }
    }
}

impl LineTable {
    /// The lines of `source` that `terminators` end, in an index of their own.
    pub fn new(source: &str, terminators: Terminators) -> Self {
        Self::own(source.as_bytes(), terminators, false)
    }

    /// `new`, of bytes not known to be text (a template's).
    pub fn of_bytes(source: &[u8], terminators: Terminators) -> Self {
        Self::own(source, terminators, false)
    }

    /// `new`, its index in a slab's room where the device reads tapes (decision 0353 step 4d: the views' reads find
    /// it there), else its own.
    pub fn placed(source: &str, terminators: Terminators) -> Self {
        Self::own(source.as_bytes(), terminators, true)
    }

    fn own(bytes: &[u8], terminators: Terminators, placed: bool) -> Self {
        use rerail_tape_format::newlines;
        let len = u32::try_from(bytes.len()).expect("a file within 4 GB");
        let mut ends = Vec::new();
        terminators.each(bytes, |at, length| ends.push((at + length - 1) as u64));
        let (u, m) = (u64::from(len), ends.len() as u64);
        let fill = |out: &mut [u8]| {
            out.fill(0);
            newlines::index_in(out, u, m, ends.iter().copied());
            Ok(())
        };
        let room = newlines::index_len(u, m) + 8;
        let home = if placed {
            crate::code_rule_dsl::tape::placed::place(room, fill).expect("an index's room")
        } else {
            let mut v = vec![0u8; room];
            fill(&mut v).expect("an index");
            rerail_tape_format::home::Home::from(v)
        };
        Self { home: Index::Own(home), at: 0, len }
    }

    /// The lines of a document whose format 16 (or 16z) tape holds them, made with its language's terminators: the
    /// tape's index, read where it lies in the tape (`Tape16::lines_at`), the tape held.
    pub fn of_f16(t: &std::sync::Arc<rerail_tape_format::f16::Tape16>) -> Self {
        Self { at: t.lines_at() as u32, len: t.source_len() as u32, home: Index::F16(t.clone()) }
    }
    pub fn of_f16z(t: &std::sync::Arc<rerail_tape_format::f16::Tape16Z>) -> Self {
        Self { at: t.lines_at() as u32, len: t.source_len() as u32, home: Index::F16Z(t.clone()) }
    }

    /// The index's bytes.
    pub fn index(&self) -> &[u8] {
        let index = &self.home[self.at as usize..];
        &index[..rerail_tape_format::newlines::len_of(index).expect("an index")]
    }

    /// The room the index is in, if it is placed (its tape's, or its own).
    pub fn room(&self) -> Option<&std::sync::Arc<dyn rerail_tape_format::home::Room>> {
        self.home.home().room()
    }

    /// The view the tape's reads of the index take: its start, in its home's bytes.
    fn view(&self) -> rerail_clif::view::TapeView {
        rerail_clif::view::TapeView { lines: self.at, source_len: self.len, ..Default::default() }
    }

    /// The 0-based line holding `byte`; a terminator's bytes belong to the line they end.
    pub fn line(&self, byte: usize) -> usize {
        rerail_tape_format::reads::f16::line(&self.home, &self.view(), byte.min(self.len as usize) as u32) as usize
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
        std::mem::size_of::<Self>() + if let Index::Own(h) = &self.home { h.len() } else { 0 }
    }

    /// The first byte of `line`.
    pub fn start(&self, line: usize) -> usize {
        rerail_tape_format::reads::f16::line_start(&self.home, &self.view(), line as u32) as usize
    }

    /// Each line's first byte, in order.
    pub fn starts(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.count()).map(|l| self.start(l) as u32)
    }

    /// Where `line`'s text ends in `source` (the file's): its terminator's first byte, or the file's end.
    pub fn end(&self, line: usize, source: &[u8]) -> usize {
        if line + 1 >= self.count() {
            return self.len as usize;
        }
        let last = self.start(line + 1) - 1;
        last + 1 - rerail_tape_format::newlines::length_ending_at(source, last)
    }

    /// Where the line after `line` starts, or the file's end on the last line (Ruff's `full_line_end`).
    pub fn full_end(&self, line: usize) -> usize {
        if line + 1 < self.count() { self.start(line + 1) } else { self.len as usize }
    }

    /// The number of lines: one more than the terminators.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn len(&self) -> usize {
        self.count()
    }

    /// The number of lines: one more than the terminators, the empty line after a final one included (core.rerail's
    /// `src_line`).
    pub fn count(&self) -> usize {
        rerail_tape_format::newlines::count(self.index()).expect("an index") as usize + 1
    }

    /// The lines a linter counts in the file (core.rerail's `line_count`: ESLint's `max-lines`, pylint's
    /// `too-many-lines`): the empty line after a final terminator left out, so an empty file has none.
    pub fn counted(&self) -> usize {
        let n = self.count();
        n - usize::from(self.start(n - 1) == self.len as usize)
    }

    /// Line `l`'s text in `source`, `[start, end)`, from 1: its first byte and its terminator's.
    pub fn text_of(&self, l: usize, source: &[u8]) -> [u32; 2] {
        [self.start(l - 1) as u32, self.end(l - 1, source) as u32]
    }

    /// A span's place, as core.rerail's `line`, `end_line` and `col` give it (one definition for every node, token
    /// and span): the line of its start, the line holding its end offset, both from 1, and its start's byte column.
    /// A terminator's bytes are the line's they end, so a span ending with one ends on the line after it.
    pub fn place(&self, [start, end]: [u32; 2]) -> (u32, u32, u32) {
        let line = self.line(start as usize);
        let last = self.line(end as usize);
        (line as u32 + 1, last as u32 + 1, start - self.start(line) as u32)
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
                .map(|line| &source[table.start(line)..table.end(line, source.as_bytes())])
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
        assert_eq!((empty.len(), empty.line(0), empty.end(0, b"")), (1, 0, 0));
        let trailing = LineTable::new("x\n", Terminators::Newlines);
        assert_eq!(
            (trailing.len(), trailing.line(2), trailing.end(1, b"x\n")),
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
            (1..=table.count()).map(|l| table.text_of(l, source.as_bytes())).map(|[a, b]| &source[a as usize..b as usize]).collect::<Vec<_>>()
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
