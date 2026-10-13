//! The map from an embedded program's bytes back to the host source they stand for, which a
//! view writes as it emits the program (Blade's compiled view; Vue's script blocks).

use std::ops::Range;
use tree_sitter::Node as TsNode;

/// An embedded program's segments, in program order: each a stretch of the program's text and
/// the host bytes it stands for.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegmentMap {
    segments: Vec<Segment>,
}

/// One stretch of the program's text and the host bytes it stands for.
#[derive(Clone, Debug)]
struct Segment {
    /// Where it starts in the program.
    at: usize,
    end: usize,
    anchor: Range<usize>,
    /// Copied from the host: each byte maps to the byte at the same offset of `anchor`.
    copied: bool,
    /// A placeholder the grammar needs and the view does not keep.
    omit: bool,
}

impl SegmentMap {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records the program bytes `at..end`, written after every earlier segment, as standing for
    /// the host bytes `anchor`: copied byte for byte (`copied`), or written over them; `omit` marks
    /// a placeholder the program's tree does not keep.
    pub(crate) fn push(
        &mut self,
        at: usize,
        end: usize,
        anchor: Range<usize>,
        copied: bool,
        omit: bool,
    ) {
        self.segments.push(Segment {
            at,
            end,
            anchor,
            copied,
            omit,
        });
    }

    /// Whether nothing was written: the host embeds no program.
    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The segment holding program byte `at` (the last one at the end of the program).
    fn segment(&self, at: usize) -> &Segment {
        let index = self
            .segments
            .partition_point(|segment| segment.end <= at)
            .min(self.segments.len() - 1);
        &self.segments[index]
    }

    /// The host position a node starting at program byte `at` starts at.
    fn start(&self, at: usize) -> usize {
        let segment = self.segment(at);
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.start
        }
    }

    /// The host position a node ending at program byte `at` ends at.
    fn end(&self, at: usize) -> usize {
        let segment = self.segment(at.saturating_sub(1));
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.end
        }
    }

    /// The host bytes a node of the program stands for.
    pub(crate) fn span(&self, node: TsNode<'_>) -> Range<usize> {
        let range = node.byte_range();
        if range.is_empty() {
            let at = self.start(range.start);
            return at..at;
        }
        let start = self.start(range.start);
        start..self.end(range.end).max(start)
    }

    /// Whether a node is part of a placeholder the program does not keep.
    pub(crate) fn omitted(&self, node: TsNode<'_>) -> bool {
        let range = node.byte_range();
        !range.is_empty() && {
            let segment = self.segment(range.start);
            segment.omit && range.end <= segment.end
        }
    }

    /// The host ranges the program copies byte for byte, in program order.
    #[cfg(test)]
    pub(crate) fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.copied)
            .map(|segment| segment.anchor.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::SegmentMap;
    use ast_grep_language::{LanguageExt, SupportLang};

    /// A copied stretch maps byte for byte; a written one stands for its whole anchor; a node
    /// inside an omitted placeholder is omitted.
    #[test]
    fn a_program_node_spans_the_host_bytes_its_segments_stand_for() {
        let mut map = SegmentMap::new();
        map.push(0, 4, 10..14, true, false);
        map.push(4, 8, 20..21, false, true);
        map.push(8, 11, 30..33, true, false);
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&SupportLang::JavaScript.get_ts_language())
            .unwrap();
        let tree = parser.parse("a=1;b=2;c=3", None).unwrap();
        let statement = |index| tree.root_node().named_child(index).unwrap();
        // `a=1;` is copied: 10..14.
        assert_eq!(map.span(statement(0)), 10..14);
        assert!(!map.omitted(statement(0)));
        // `b=2;` is written over 20..21 and omitted.
        assert_eq!(map.span(statement(1)), 20..21);
        assert!(map.omitted(statement(1)));
        // `c=3` is copied from 30.
        assert_eq!(map.span(statement(2)), 30..33);
        // The whole program runs from the first anchor to the last copied byte.
        assert_eq!(map.span(tree.root_node()), 10..33);
        assert_eq!(map.copied().collect::<Vec<_>>(), [10..14, 30..33]);
    }
}
