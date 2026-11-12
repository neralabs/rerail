//! A Python string literal's decoded text with the source span of every decoded character, so a
//! node parsed from the text maps back to the bytes it came from.

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


pub(crate) fn push(decoded: &mut Decoded, c: char, start: usize, end: usize) {
    decoded.spans.push(Span {
        at: decoded.text.len() as u32,
        start: start as u32,
        end: end as u32,
    });
    decoded.text.push(c);
}



#[cfg(test)]
mod tests;
