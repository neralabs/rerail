//! The map from an embedded program's bytes back to the host source they stand for, which a
//! view writes as it emits the program (Blade's compiled view; Vue's script blocks). The program
//! is a document of its own, its nodes spanning its own text; the map says which host bytes a
//! stretch of that text stands for, and is kept with the program's tape (`tape::GuestImage`).

use std::ops::Range;

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

    /// Builds the hull tables `host_span` reads; segments pushed after are not covered.
    pub(crate) fn index(&mut self) {
        let len = self.segments.len();
        self.starts = self.segments.iter().map(|segment| segment.from).collect();
        self.ends = self.segments.iter().map(|segment| segment.to).collect();
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

    /// The host bytes the program bytes `range` stand for: the least range holding every host
    /// byte a segment it overlaps stands for (a copied segment's overlapped part, a written one's
    /// whole anchor). The program's text need not stand in host order (a Vue template expression
    /// may precede the script it reads), so the range may hold host bytes its middle does not
    /// stand for. An empty range maps to the position its byte maps to.
    pub(crate) fn host_span(&self, range: Range<usize>) -> Range<usize> {
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
        let part = |index: usize| -> (u32, u32) {
            let segment = &self.segments[index];
            if segment.copied {
                (
                    self.point(index, start.max(segment.at)),
                    self.point(index, end.min(segment.end)),
                )
            } else {
                (segment.from, segment.to)
            }
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
        low as usize..high.max(low) as usize
    }

    /// The host bytes the program bytes `range` copy, where one copied segment holds them all.
    pub(crate) fn copied_span(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = u32::try_from(range.start).ok()?;
        let end = u32::try_from(range.end).ok()?;
        let index = self.segment(start)?;
        let segment = &self.segments[index];
        (segment.copied && segment.at <= start && end <= segment.end)
            .then(|| self.point(index, start) as usize..self.point(index, end) as usize)
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
    /// that a copied one is as long as its anchor; indexed for `host_span`.
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

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.segments.capacity() * std::mem::size_of::<Segment>()
            + (self.starts.capacity() + self.ends.capacity()) * std::mem::size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::SegmentMap;

    /// A copied stretch maps byte for byte; a written one stands for its whole anchor; a range
    /// over several stands for the least host range holding all they stand for, in any order; a
    /// stretch inside an omitted placeholder is omitted.
    #[test]
    fn program_bytes_span_the_host_bytes_their_segments_stand_for() {
        let mut map = SegmentMap::new();
        // `a=1;b=2;c=3;d`: `a=1;` copied from 10, `b=2;` written over 20..21 and omitted, `c=3;`
        // copied from 30, `d` copied from 5 (before the rest).
        map.push(0, 4, 10..14, true, false);
        map.push(4, 8, 20..21, false, true);
        map.push(8, 12, 30..34, true, false);
        map.push(12, 13, 5..6, true, false);
        map.index();
        assert_eq!(map.host_span(0..4), 10..14);
        assert_eq!(map.host_span(1..3), 11..13);
        assert!(!map.omitted(0..4));
        assert_eq!(map.host_span(4..8), 20..21);
        assert!(map.omitted(4..8));
        assert_eq!(map.host_span(8..11), 30..33);
        assert_eq!(map.host_span(2..10), 12..32);
        assert_eq!(map.host_span(0..13), 5..34);
        assert_eq!(map.host_span(5..5), 20..20);
        assert_eq!(map.host_span(9..9), 31..31);
        assert_eq!(map.copied_span(8..11), Some(30..33));
        assert_eq!(map.copied_span(2..10), None);
        assert_eq!(map.copied_span(4..6), None);
        let mut bytes = Vec::new();
        map.write(&mut bytes);
        assert_eq!(SegmentMap::read(&bytes, 13, 34), Ok(map));
        assert!(SegmentMap::read(&bytes, 12, 34).is_err());
        assert!(SegmentMap::read(&bytes, 13, 33).is_err());
    }
}
