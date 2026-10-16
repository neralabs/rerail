//! The map from an embedded program's bytes back to the host source they stand for, which a
//! view writes as it emits the program (Blade's compiled view; Vue's script blocks). The program
//! is a document of its own, its nodes spanning its own text; the map says which host bytes a
//! stretch of that text stands for (`file_ranges`). It is the document's, beside its tape, which
//! holds the program's tree alone (`tape::DerivedImage`; stored in `file_documents`).
//!
//! A program need not stand in host order (a Vue template expression may precede the script it
//! reads), so a stretch of it may stand for several host ranges: the map keeps them apart, never
//! widening them to one range holding bytes the stretch does not stand for. Positions are
//! half-open: a program position at the boundary of two segments belongs to the one starting
//! there, the program's end to its last segment.

use std::ops::Range;

/// The host bytes a stretch of a program stands for (`SegmentMap::file_ranges`): disjoint ranges
/// in file order, none touching the next, and the primary one, which holds the bytes the
/// stretch's first segment stands for: where the stretch starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileRanges {
    /// The first range in file order, and the rest after it.
    head: Range<usize>,
    tail: Vec<Range<usize>>,
    /// The primary range's index in file order.
    primary: usize,
}

impl FileRanges {
    pub(crate) fn one(range: Range<usize>) -> Self {
        Self {
            head: range,
            tail: Vec::new(),
            primary: 0,
        }
    }

    /// The union of `pieces`, the first of which stands for where the stretch starts: one range
    /// per run of pieces that overlap or touch.
    fn union(mut pieces: Vec<Range<usize>>) -> Self {
        let start = pieces[0].clone();
        pieces.sort_by_key(|piece| (piece.start, piece.end));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match merged.last_mut() {
                Some(last) if piece.start <= last.end => last.end = last.end.max(piece.end),
                _ => merged.push(piece),
            }
        }
        let primary = merged
            .iter()
            .position(|range| range.start <= start.start && start.end <= range.end)
            .expect("the union holds every piece");
        let mut ranges = merged.into_iter();
        Self {
            head: ranges.next().expect("a piece"),
            tail: ranges.collect(),
            primary,
        }
    }

    /// The ranges with the white space between two of them joined: a node of a tree stands for its
    /// tokens and the white space between them, and so does a program's, where its tokens stand
    /// apart in the host only by white space (Blade's `echo ` over `{{`, `$x` copied, `;` over
    /// `}}`: one range, `{{ $x }}`). Anything else between them keeps them apart.
    pub(crate) fn bridged(self, host: &str) -> Self {
        if self.tail.is_empty() {
            return self;
        }
        let blank = |gap: Range<usize>| {
            host.get(gap)
                .is_some_and(|text| text.bytes().all(|byte| byte.is_ascii_whitespace()))
        };
        let mut ranges: Vec<Range<usize>> = Vec::with_capacity(self.tail.len() + 1);
        let mut primary = 0;
        for (index, range) in self.iter().enumerate() {
            match ranges.last_mut() {
                Some(last) if blank(last.end..range.start) => last.end = range.end,
                _ => ranges.push(range.clone()),
            }
            if index == self.primary {
                primary = ranges.len() - 1;
            }
        }
        let mut ranges = ranges.into_iter();
        Self {
            head: ranges.next().expect("a range"),
            tail: ranges.collect(),
            primary,
        }
    }

    /// The range a single span reports: the one where the stretch starts. A stretch standing for
    /// several ranges reports that one, never a range holding bytes between them.
    pub(crate) fn primary(&self) -> Range<usize> {
        match self.primary {
            0 => self.head.clone(),
            index => self.tail[index - 1].clone(),
        }
    }

    /// The one range the stretch stands for, where it stands for one.
    pub(crate) fn single(&self) -> Option<Range<usize>> {
        self.tail.is_empty().then(|| self.head.clone())
    }

    /// Each range, in file order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Range<usize>> {
        std::iter::once(&self.head).chain(&self.tail)
    }
}

/// Whether `outer` holds `inner`, positions half-open: a zero-width `inner` at a boundary belongs
/// to the range starting there, and `end` (the end of the text both lie in) to the range ending
/// there.
pub(crate) fn holds(outer: &Range<usize>, inner: &Range<usize>, end: usize) -> bool {
    if inner.is_empty() {
        outer.start <= inner.start
            && (inner.start < outer.end || (inner.start == outer.end && inner.start == end))
    } else {
        outer.start <= inner.start && inner.end <= outer.end
    }
}

/// An embedded program's segments, in program order: each a stretch of the program's text and
/// the host bytes it stands for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SegmentMap {
    segments: Vec<Segment>,
    /// Over `segments`, the least anchor start and the greatest anchor end of each run of
    /// `2^level` segments from each index (`hull`): level `level` at `level * len`.
    starts: Vec<u32>,
    ends: Vec<u32>,
}

/// One stretch of the program's text and the host bytes it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    /// Where it starts and ends in the program.
    at: u32,
    end: u32,
    /// The host bytes it stands for.
    from: u32,
    to: u32,
    /// Copied from the host: each byte maps to the byte at the same offset of the anchor.
    copied: bool,
    /// A placeholder the grammar needs and the view does not keep.
    omit: bool,
}

/// A segment's persisted width: four `u32`s and a flag byte.
const SEGMENT_BYTES: usize = 17;

impl SegmentMap {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records the program bytes `at..end`, written after every earlier segment, as standing for
    /// the host bytes `anchor`: copied byte for byte (`copied`), or written over them; `omit` marks
    /// a placeholder the program's tree does not keep. Positions past `u32` are clamped: no tape
    /// holds a source that long (`tape` refuses it).
    pub(crate) fn push(
        &mut self,
        at: usize,
        end: usize,
        anchor: Range<usize>,
        copied: bool,
        omit: bool,
    ) {
        let narrow = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        self.segments.push(Segment {
            at: narrow(at),
            end: narrow(end),
            from: narrow(anchor.start),
            to: narrow(anchor.end),
            copied,
            omit,
        });
    }

    /// Whether nothing was written: the host embeds no program.
    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Builds the hull tables `bounds` reads; segments pushed after are not covered.
    pub(crate) fn index(&mut self) {
        let len = self.segments.len();
        // A segment of no program or host bytes stands for none: it bounds nothing.
        let empty = |segment: &Segment| segment.at == segment.end || segment.from == segment.to;
        self.starts = self
            .segments
            .iter()
            .map(|segment| {
                if empty(segment) {
                    u32::MAX
                } else {
                    segment.from
                }
            })
            .collect();
        self.ends = self
            .segments
            .iter()
            .map(|segment| if empty(segment) { 0 } else { segment.to })
            .collect();
        let mut width = 1;
        while width * 2 <= len {
            let (previous, next) = (self.starts.len() - len, self.starts.len());
            for index in 0..len {
                let (start, end) = if index + width < len {
                    (
                        self.starts[previous + index].min(self.starts[previous + index + width]),
                        self.ends[previous + index].max(self.ends[previous + index + width]),
                    )
                } else {
                    (self.starts[previous + index], self.ends[previous + index])
                };
                self.starts.push(start);
                self.ends.push(end);
            }
            debug_assert_eq!(self.starts.len(), next + len);
            width *= 2;
        }
    }

    /// The least anchor start and the greatest anchor end of segments `from..to` (not empty).
    fn hull(&self, from: usize, to: usize) -> (u32, u32) {
        let len = self.segments.len();
        let level = (usize::BITS - 1 - (to - from).leading_zeros()) as usize;
        let row = level * len;
        let last = to - (1 << level);
        (
            self.starts[row + from].min(self.starts[row + last]),
            self.ends[row + from].max(self.ends[row + last]),
        )
    }

    /// The index of the segment holding program byte `at`, or the last one before it.
    fn segment(&self, at: u32) -> Option<usize> {
        self.segments
            .partition_point(|segment| segment.end <= at)
            .min(self.segments.len().checked_sub(1)?)
            .into()
    }

    /// The host position program byte `at` maps to in segment `index`: its copy, or the start of
    /// the bytes a written segment stands for.
    fn point(&self, index: usize, at: u32) -> u32 {
        let segment = &self.segments[index];
        if segment.copied {
            segment.from + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.from
        }
    }

    /// The host ranges the program bytes `range` stand for, in program order: a copied segment's
    /// overlapped part, a written one's whole anchor, each run whose host bytes follow one another
    /// merged (`FileRanges`). An empty range stands for the position its program position maps to
    /// (half-open: the segment starting there; the program's end, the last segment's end).
    pub(crate) fn file_ranges(&self, range: Range<usize>) -> FileRanges {
        let narrow = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        let (start, end) = (narrow(range.start), narrow(range.end));
        let Some(first) = self.segment(start) else {
            return FileRanges::one(range);
        };
        let point = |index: usize, at: u32| self.point(index, at) as usize;
        if start == end {
            let at = point(first, start);
            return FileRanges::one(at..at);
        }
        let last = self.segments.partition_point(|segment| segment.at < end);
        // One piece per segment the range overlaps; a piece of no host bytes adds none. A written
        // token's anchor may hold the copied pieces beside it (a `(` written over the whole
        // attribute whose value is copied inside it): the bytes are their union.
        // Until a piece falls outside the first, the union is the first: no list is built.
        let mut head: Option<Range<usize>> = None;
        let mut pieces = Vec::new();
        for index in first..last {
            let segment = &self.segments[index];
            // A segment of no program bytes stands for nothing of the range's.
            if segment.at == segment.end {
                continue;
            }
            let piece = if segment.copied {
                point(index, start.max(segment.at))..point(index, end.min(segment.end))
            } else {
                segment.from as usize..segment.to as usize
            };
            if piece.is_empty() {
                continue;
            }
            if !pieces.is_empty() {
                pieces.push(piece);
                continue;
            }
            match &head {
                None => head = Some(piece),
                Some(head) if head.start <= piece.start && piece.end <= head.end => {}
                Some(head) => {
                    pieces.push(head.clone());
                    pieces.push(piece);
                }
            }
        }
        match (head, pieces.is_empty()) {
            (None, _) => {
                let at = point(first, start);
                FileRanges::one(at..at)
            }
            (Some(piece), true) => FileRanges::one(piece),
            (Some(_), false) => FileRanges::union(pieces),
        }
    }

    /// Whether the program bytes `range` stand for some host byte: whether a segment they overlap
    /// gives a piece of width (`file_ranges`).
    pub(crate) fn stands_for(&self, range: Range<usize>) -> bool {
        let narrow = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        let (start, end) = (narrow(range.start), narrow(range.end));
        if start >= end {
            return false;
        }
        let first = self
            .segments
            .partition_point(|segment| segment.end <= start);
        self.segments[first..]
            .iter()
            .take_while(|segment| segment.at < end)
            .any(|segment| {
                if segment.copied {
                    segment.at.max(start) < segment.end.min(end)
                } else {
                    segment.at < segment.end && segment.from < segment.to
                }
            })
    }

    /// A bound on the host bytes the program bytes `range` stand for: the least range holding all
    /// of them, which may hold host bytes they do not stand for. An interval holds every one of
    /// their bytes exactly when it holds this range, so it places them inside a host node and
    /// prunes a search; it is never the bytes they stand for (`file_ranges`).
    pub(crate) fn bounds(&self, range: Range<usize>) -> Range<usize> {
        let narrow = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        let (start, end) = (narrow(range.start), narrow(range.end));
        let Some(first) = self.segment(start) else {
            return range;
        };
        // The segments the range overlaps: from the one holding its first byte to the last one
        // starting before its end.
        let last = self.segments.partition_point(|segment| segment.at < end);
        if start == end || last <= first {
            let at = self.point(first, start) as usize;
            return at..at;
        }
        let last = last - 1;
        // As `file_ranges` reads the segments: a piece of no host bytes bounds nothing.
        let part = |index: usize| -> (u32, u32) {
            let segment = &self.segments[index];
            let (from, to) = if segment.copied {
                (
                    self.point(index, start.max(segment.at)),
                    self.point(index, end.min(segment.end)),
                )
            } else {
                (segment.from, segment.to)
            };
            if from < to { (from, to) } else { (u32::MAX, 0) }
        };
        let (mut low, mut high) = part(first);
        if last > first {
            let (from, to) = part(last);
            low = low.min(from);
            high = high.max(to);
        }
        if last > first + 1 {
            let (from, to) = self.hull(first + 1, last);
            low = low.min(from);
            high = high.max(to);
        }
        if low > high {
            let at = self.point(first, start) as usize;
            return at..at;
        }
        low as usize..high as usize
    }

    /// The host bytes the program bytes `range` copy, where one copied segment holds them all. An
    /// empty range at a boundary is the segment's starting there, or, where that one is written,
    /// the copied one's ending there: an insertion right after copied text.
    pub(crate) fn copied_span(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = u32::try_from(range.start).ok()?;
        let end = u32::try_from(range.end).ok()?;
        let index = self.segment(start)?;
        let copies = |index: usize| {
            let segment = &self.segments[index];
            (segment.copied && segment.at <= start && end <= segment.end)
                .then(|| self.point(index, start) as usize..self.point(index, end) as usize)
        };
        copies(index).or_else(|| {
            (start == end && index > 0 && self.segments[index - 1].end == start)
                .then(|| copies(index - 1))
                .flatten()
        })
    }

    /// Whether some byte of the program bytes `range` is copied from the host.
    pub(crate) fn copies(&self, range: Range<usize>) -> bool {
        let (Ok(start), Ok(end)) = (u32::try_from(range.start), u32::try_from(range.end)) else {
            return false;
        };
        let first = self
            .segments
            .partition_point(|segment| segment.end <= start);
        self.segments[first..]
            .iter()
            .take_while(|segment| segment.at < end)
            .any(|segment| segment.copied && segment.at < segment.end)
    }

    /// Whether the program bytes `range` (not empty) lie in a placeholder the program's tree does
    /// not keep.
    pub(crate) fn omitted(&self, range: Range<usize>) -> bool {
        !range.is_empty()
            && self.segment(range.start as u32).is_some_and(|index| {
                let segment = &self.segments[index];
                segment.omit
                    && segment.at as usize <= range.start
                    && range.end <= segment.end as usize
            })
    }

    /// The program ranges of the placeholders its tree does not keep, in program order.
    pub(crate) fn omissions(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.omit)
            .map(|segment| segment.at as usize..segment.end as usize)
    }

    /// The host ranges the program copies byte for byte, in program order.
    #[cfg(test)]
    pub(crate) fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.copied)
            .map(|segment| segment.from as usize..segment.to as usize)
    }

    /// Each copied stretch: its program bytes and the host bytes they copy, in program order.
    #[cfg(test)]
    pub(crate) fn copied_stretches(
        &self,
    ) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.copied)
            .map(|segment| {
                (
                    segment.at as usize..segment.end as usize,
                    segment.from as usize..segment.to as usize,
                )
            })
    }

    pub(crate) fn len(&self) -> usize {
        self.segments.len()
    }

    /// Appends the segments' persisted form: per segment its program and host ranges, then a byte
    /// of flags (1: copied, 2: omitted).
    pub(crate) fn write(&self, bytes: &mut Vec<u8>) {
        for segment in &self.segments {
            for value in [segment.at, segment.end, segment.from, segment.to] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.push(u8::from(segment.copied) | (u8::from(segment.omit) << 1));
        }
    }

    /// The bytes `count` segments take (`write`).
    pub(crate) fn persisted_len(count: usize) -> Option<usize> {
        count.checked_mul(SEGMENT_BYTES)
    }

    /// Reads `count` segments `write` wrote over a program of `program_len` bytes and a host of
    /// `host_len`, checking that they follow one another in program order, lie in both texts, and
    /// that a copied one is as long as its anchor; indexed for `bounds`.
    pub(crate) fn read(bytes: &[u8], program_len: usize, host_len: usize) -> Result<Self, String> {
        if !bytes.len().is_multiple_of(SEGMENT_BYTES) {
            return Err("truncated embedded program segments".into());
        }
        let mut segments = Vec::with_capacity(bytes.len() / SEGMENT_BYTES);
        let mut previous = 0u32;
        for chunk in bytes.chunks_exact(SEGMENT_BYTES) {
            let word = |index: usize| {
                u32::from_le_bytes(chunk[index * 4..index * 4 + 4].try_into().expect("u32"))
            };
            let segment = Segment {
                at: word(0),
                end: word(1),
                from: word(2),
                to: word(3),
                copied: chunk[16] & 1 != 0,
                omit: chunk[16] & 2 != 0,
            };
            if chunk[16] > 3
                || segment.at < previous
                || segment.end < segment.at
                || segment.end as usize > program_len
                || segment.to < segment.from
                || segment.to as usize > host_len
                || (segment.copied && segment.to - segment.from != segment.end - segment.at)
            {
                return Err("noncanonical embedded program segments".into());
            }
            previous = segment.end;
            segments.push(segment);
        }
        let mut map = Self {
            segments,
            ..Self::default()
        };
        map.index();
        Ok(map)
    }

    /// Checks a map read back against the texts it links (`read` checks only its own shape): every
    /// segment starts and ends on character boundaries of both, and a copied one's bytes are the
    /// host bytes it stands for. A map that fails is no map of these texts.
    pub(crate) fn check(&self, program: &str, host: &str) -> Result<(), String> {
        let fits = self.segments.iter().all(|segment| {
            let (at, end) = (segment.at as usize, segment.end as usize);
            let (from, to) = (segment.from as usize, segment.to as usize);
            program.is_char_boundary(at)
                && program.is_char_boundary(end)
                && host.is_char_boundary(from)
                && host.is_char_boundary(to)
                && (!segment.copied || program.as_bytes()[at..end] == host.as_bytes()[from..to])
        });
        if fits {
            Ok(())
        } else {
            Err("embedded program segments do not fit the texts they link".into())
        }
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.segments.capacity() * std::mem::size_of::<Segment>()
            + (self.starts.capacity() + self.ends.capacity()) * std::mem::size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::SegmentMap;

    /// A copied stretch maps byte for byte; a written one stands for its whole anchor; a range
    /// over several stands for each run of host bytes they stand for, in file order, never a range
    /// holding bytes between them; a stretch inside an omitted placeholder is omitted.
    #[test]
    fn program_bytes_stand_for_the_host_ranges_of_their_segments() {
        let mut map = SegmentMap::new();
        // `a=1;b=2;c=3;d`: `a=1;` copied from 10, `b=2;` written over 20..21 and omitted, `c=3;`
        // copied from 30, `d` copied from 5 (before the rest).
        map.push(0, 4, 10..14, true, false);
        map.push(4, 8, 20..21, false, true);
        map.push(8, 12, 30..34, true, false);
        map.push(12, 13, 5..6, true, false);
        map.index();
        let ranges = |range: std::ops::Range<usize>| {
            map.file_ranges(range).iter().cloned().collect::<Vec<_>>()
        };
        assert_eq!(map.file_ranges(0..4).single(), Some(10..14));
        assert_eq!(map.file_ranges(1..3).single(), Some(11..13));
        assert!(!map.omitted(0..4));
        assert_eq!(map.file_ranges(4..8).single(), Some(20..21));
        assert!(map.omitted(4..8));
        assert_eq!(map.file_ranges(8..11).single(), Some(30..33));
        assert_eq!(ranges(2..10), [12..14, 20..21, 30..32]);
        assert_eq!(map.file_ranges(2..10).primary(), 12..14);
        assert_eq!(map.file_ranges(2..10).single(), None);
        // In file order; the primary range is where the stretch starts.
        assert_eq!(ranges(0..13), [5..6, 10..14, 20..21, 30..34]);
        assert_eq!(map.file_ranges(0..13).primary(), 10..14);
        // A bound only: it holds bytes the range does not stand for.
        assert_eq!(map.bounds(2..10), 12..32);
        // Zero-width positions are half-open: a boundary belongs to the segment starting there,
        // the program's end to the last segment.
        assert_eq!(map.file_ranges(5..5).single(), Some(20..20));
        assert_eq!(map.file_ranges(9..9).single(), Some(31..31));
        assert_eq!(map.file_ranges(4..4).single(), Some(20..20));
        assert_eq!(map.file_ranges(8..8).single(), Some(30..30));
        assert_eq!(map.file_ranges(13..13).single(), Some(6..6));
        assert_eq!(map.copied_span(8..11), Some(30..33));
        // An insertion at the end of copied text, before written text, is the copied text's end.
        assert_eq!(map.copied_span(4..4), Some(14..14));
        assert_eq!(map.copied_span(12..12), Some(5..5));
        // A written token's anchor may hold the copied bytes beside it (Vue's `(` and `)` written
        // over the attribute whose value is copied inside them): one range, the anchor.
        let mut wrapped = SegmentMap::new();
        wrapped.push(0, 1, 36..49, false, false);
        wrapped.push(1, 5, 44..48, true, false);
        wrapped.push(5, 6, 36..49, false, false);
        wrapped.index();
        assert_eq!(wrapped.file_ranges(0..6).single(), Some(36..49));
        assert_eq!(wrapped.file_ranges(1..5).single(), Some(44..48));
        assert_eq!(wrapped.bounds(0..6), 36..49);
        assert_eq!(map.copied_span(2..10), None);
        assert_eq!(map.copied_span(4..6), None);
        let mut bytes = Vec::new();
        map.write(&mut bytes);
        assert_eq!(SegmentMap::read(&bytes, 13, 34), Ok(map));
        assert!(SegmentMap::read(&bytes, 12, 34).is_err());
        assert!(SegmentMap::read(&bytes, 13, 33).is_err());
    }

    /// A map read back fits the texts it links, or is refused: a copied stretch is the host bytes
    /// it stands for, and every segment starts and ends on a character of both.
    #[test]
    fn a_map_that_does_not_fit_its_texts_is_refused() {
        let mut map = SegmentMap::new();
        map.push(0, 2, 3..5, true, false);
        map.push(2, 3, 5..7, false, false);
        map.index();
        assert_eq!(map.check("ab;", "xyzabé"), Ok(()));
        // The copied bytes differ from the host's.
        assert!(map.check("ac;", "xyzabé").is_err());
        // The written segment ends inside the host's `é`.
        let mut split = SegmentMap::new();
        split.push(0, 2, 3..5, true, false);
        split.push(2, 3, 5..6, false, false);
        split.index();
        assert!(split.check("ab;", "xyzabé").is_err());
    }
}
