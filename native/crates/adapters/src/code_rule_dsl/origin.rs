//! Where a document's bytes come from. A file's own tree is the file; an embedded program (`embed`)
//! stands for the file bytes its segments map it to; a literal's parsed tree (`parse`) for the
//! characters of the literal it decodes. Each such document is an `Origin` into the document it was
//! read from, and `InFile` places a document's nodes and ranges in its file.
//!
//! A node of a program written out of file order may stand for several file ranges: `place` keeps
//! them apart (`FileRanges`), and what reads the bytes a node stands for (syntax errors, a
//! suppression's cover, `host`, `embedded`) reads every one. What reports one span (a witness, a
//! capture, an export) reports the primary range, where the node starts (`FileRanges::primary`),
//! never a range widened over bytes it does not stand for. Positions are half-open
//! (`embed::segments::holds`).
//!
//! Every position a rule reports or carries across documents goes through here: witnesses,
//! captures, fixes, captures exported out of a parsed tree or an embedded program, syntax errors,
//! suppression covers, and the lines a transform counts. No caller maps a byte range itself.

use crate::code_rule_dsl::embed::segments::FileRanges;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Span, Terminators};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::ops::Range;
use std::rc::Rc;

/// How one document's bytes map into the document it was read from (its outer document).
pub trait Origin {
    /// The outer ranges the node `id`, spanning `range` here, stands for. A node's ranges hold its
    /// descendants'.
    fn of_node(&self, id: usize, range: Range<usize>) -> Option<FileRanges>;
    /// The outer ranges `range` here stands for: a regex group's, or a capture's a nested document
    /// exported here.
    fn of_range(&self, range: Range<usize>) -> Option<FileRanges>;
    /// The least outer range holding every range `of_node` gives for the node. The node's ranges
    /// lie in an interval exactly when this range does, so placing the node inside a node of the
    /// outer document, or pruning a search by it, reads this; nothing reads it as the bytes the
    /// node stands for.
    fn bounds(&self, id: usize, range: Range<usize>) -> Option<Range<usize>>;
    /// The outer bytes `range` copies byte for byte, where it does: what a fix may rewrite.
    fn copied(&self, range: Range<usize>) -> Option<Range<usize>>;
    /// Whether some byte of `range` is copied from the outer document (the rest is what the
    /// document's builder wrote).
    fn copies(&self, range: Range<usize>) -> bool {
        self.copied(range).is_some()
    }
}

/// A file's own tree: its bytes are the file's.
pub(crate) struct Same;

impl Origin for Same {
    fn of_node(&self, _: usize, range: Range<usize>) -> Option<FileRanges> {
        Some(FileRanges::one(range))
    }
    fn of_range(&self, range: Range<usize>) -> Option<FileRanges> {
        Some(FileRanges::one(range))
    }
    fn bounds(&self, _: usize, range: Range<usize>) -> Option<Range<usize>> {
        Some(range)
    }
    fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        Some(range)
    }
}

/// The lines of a file: its tape's, or read for it.
enum Lines<'t> {
    Taped(&'t LineTable),
    Read(Rc<LineTable>),
}

/// A document as its file reads it: the file's text, dialect and lines, and the document's origin
/// in it. `None` from `of` for a literal's parsed tree, whose origin is the document it was parsed
/// from (`literal::SubTree`), not a file.
pub struct InFile<'t> {
    source: &'t str,
    dialect: GrammarDialect,
    lines: Lines<'t>,
    origin: &'t dyn Origin,
}

impl<'t> InFile<'t> {
    /// The file the document of `node` is read from.
    pub fn of<D: Doc>(node: &Node<'t, D>) -> Option<Self> {
        let doc: &'t dyn std::any::Any = node.get_doc();
        if let Some(doc) = doc.downcast_ref::<TapeDoc>() {
            return Some(Self::tape(doc));
        }
        let doc = doc.downcast_ref::<StrDoc<GrammarDialect>>()?;
        let (source, dialect) = (doc.get_source().as_str(), *doc.get_lang());
        Some(Self {
            source,
            dialect,
            lines: Lines::Read(crate::code_rule_dsl::matcher::file_lines(
                source,
                crate::code_rule_dsl::lines::terminators(dialect),
            )),
            origin: &Same,
        })
    }

    /// The file a tape document is read from: its own, or for a guest its host's.
    pub fn tape(doc: &'t TapeDoc) -> Self {
        Self {
            source: doc.file_source().as_str(),
            dialect: doc.file_dialect(),
            lines: Lines::Taped(doc.file_lines()),
            origin: doc,
        }
    }

    /// The file's whole text.
    pub fn source(&self) -> &'t str {
        self.source
    }

    /// The file's dialect.
    pub fn dialect(&self) -> GrammarDialect {
        self.dialect
    }

    /// The file's lines.
    pub fn lines(&self) -> &LineTable {
        match &self.lines {
            Lines::Taped(lines) => lines,
            Lines::Read(lines) => lines,
        }
    }

    /// The file ranges a node of the document stands for.
    pub fn place<D: Doc>(&self, node: &Node<'_, D>) -> FileRanges {
        self.origin
            .of_node(node.node_id(), node.range())
            .expect("a document's node lies in its file")
    }

    /// The span a node reports: its primary file range (`FileRanges::primary`).
    pub fn node<D: Doc>(&self, node: &Node<'_, D>) -> Range<usize> {
        self.place(node).primary()
    }

    /// The file ranges a range of the document stands for.
    pub fn places(&self, range: Range<usize>) -> FileRanges {
        self.origin
            .of_range(range)
            .expect("a document's range lies in its file")
    }

    /// The span a range of the document reports: its primary file range.
    pub fn range(&self, range: Range<usize>) -> Range<usize> {
        self.places(range).primary()
    }

    /// The file bytes a range of the document copies, where it does.
    pub fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        self.origin.copied(range)
    }

    /// Whether some byte of a range of the document is the file's, copied.
    pub(crate) fn copies(&self, range: Range<usize>) -> bool {
        self.origin.copies(range)
    }

    /// Whether a node's text is the file bytes it stands for, ASCII letters in any case: a copied
    /// node's is, and so is one written over the same text (Blade's `@isset(` `isset(`). A node
    /// standing for several file ranges is not.
    pub(crate) fn spells<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        self.place(node)
            .single()
            .and_then(|range| self.source.get(range))
            .is_some_and(|file| file.eq_ignore_ascii_case(&node.text()))
    }

    /// The file's text a node of the document stands for, where it stands for one range; else the
    /// node's own text (a program's), never the file's bytes between its ranges.
    pub fn text<D: Doc>(&self, node: &Node<'t, D>) -> Cow<'t, str> {
        match self
            .place(node)
            .single()
            .and_then(|range| self.source.get(range))
        {
            Some(text) => Cow::Borrowed(text),
            None => node.text(),
        }
    }

    /// The diagnostic span of file bytes `range`.
    pub fn span(&self, range: Range<usize>) -> Span {
        self.lines().span(range.start, range.end)
    }
}

/// The program bytes guest node `id` (over its span `range`) stands for in its host: from the first of its tokens
/// that stands for host bytes to the last (`SegmentMap::stands_for`); a node none of whose tokens does, from its first
/// token to its last; a node with none, its span. Its tokens are its stored leaves and, in the gaps they leave in its
/// span, the runs of text that are not white space: the implied tokens the tape stores no node for (RER-131: Blade's
/// `echo`, the `;` Vue's programs start with). A node's span may run past them: a root's holds the white space after
/// its last statement, which stands for nothing of the node's. Read off the tape from each end until a token stands,
/// never stored. `None` past the tape's nodes.
fn extent(doc: &TapeDoc, segments: &crate::code_rule_dsl::embed::segments::SegmentMap, id: usize, range: Range<usize>) -> Option<Range<usize>> {
    let id = u32::try_from(id).ok().filter(|&id| (id as usize) < doc.transducer_len())?;
    let text = doc.get_source().as_bytes();
    let end = doc.transducer_descendants(id).end;
    let leaf = |i: u32| doc.transducer_descendants(i).end == i + 1;
    let white = |at: usize| text.get(at).is_none_or(u8::is_ascii_whitespace);
    // Its tokens from the front: each leaf, after the runs in the gap before it.
    let mut front = None::<Range<usize>>;
    let mut first_any = None::<Range<usize>>;
    let mut at = range.start;
    let mut take = |token: Range<usize>, first_any: &mut Option<Range<usize>>| -> bool {
        first_any.get_or_insert(token.clone());
        segments.stands_for(token.clone()).then(|| front = Some(token)).is_some()
    };
    'front: for i in (id..end).filter(|&i| leaf(i)).map(Some).chain([None]) {
        let (gap_end, token) = match i {
            Some(i) => { let span = doc.transducer_span(i); (span.start.max(at), Some(span)) }
            None => (range.end, None),
        };
        let mut from = at;
        while from < gap_end {
            if white(from) { from += 1; continue; }
            let to = (from..gap_end).find(|&b| white(b)).unwrap_or(gap_end);
            if take(from..to, &mut first_any) { break 'front; }
            from = to;
        }
        if let Some(span) = token {
            at = at.max(span.end);
            if span.start < span.end && take(span, &mut first_any) { break; }
        }
    }
    // Its tokens from the back: each leaf, before the runs in the gap after it.
    let mut back = None::<Range<usize>>;
    let mut last_any = None::<Range<usize>>;
    let mut at = range.end;
    let mut take = |token: Range<usize>, last_any: &mut Option<Range<usize>>| -> bool {
        last_any.get_or_insert(token.clone());
        segments.stands_for(token.clone()).then(|| back = Some(token)).is_some()
    };
    'back: for i in (id..end).rev().filter(|&i| leaf(i)).map(Some).chain([None]) {
        let (gap_start, token) = match i {
            Some(i) => { let span = doc.transducer_span(i); (span.end.min(at), Some(span)) }
            None => (range.start, None),
        };
        let mut to = at;
        while to > gap_start {
            if white(to - 1) { to -= 1; continue; }
            let from = (gap_start..to).rev().find(|&b| white(b)).map_or(gap_start, |b| b + 1);
            if take(from..to, &mut last_any) { break 'back; }
            to = from;
        }
        if let Some(span) = token {
            at = at.min(span.start);
            if span.start < span.end && take(span, &mut last_any) { break; }
        }
    }
    let (start, stop) = match (front, back, first_any, last_any) {
        (Some(front), Some(back), _, _) => (front.start, back.end),
        (_, _, Some(first), Some(last)) => (first.start, last.end),
        _ => return Some(range),
    };
    Some(if start < stop { start..stop } else { range })
}

/// A host's bytes are the file's; a guest's program text stands for the file ranges its segments
/// map it to (`SegmentMap::file_ranges`), and a guest node for those of its extent, from its first
/// byte standing for file bytes to its last (`SegmentMap::extent`), white space between its ranges
/// included, as a tree's node holds the white space between its tokens (`FileRanges::bridged`).
impl Origin for TapeDoc {
    fn of_node(&self, id: usize, range: Range<usize>) -> Option<FileRanges> {
        Some(match &self.host {
            None => FileRanges::one(range),
            Some(link) => {
                let extent = extent(self, &link.segments, id, range)?;
                link.segments
                    .file_ranges(extent)
                    .bridged(&link.host.source)
            }
        })
    }

    fn of_range(&self, range: Range<usize>) -> Option<FileRanges> {
        Some(match &self.host {
            None => FileRanges::one(range),
            Some(link) => link.segments.file_ranges(range).bridged(&link.host.source),
        })
    }

    fn bounds(&self, id: usize, range: Range<usize>) -> Option<Range<usize>> {
        Some(match &self.host {
            None => range,
            Some(link) => {
                let extent = extent(self, &link.segments, id, range)?;
                link.segments.bounds(extent)
            }
        })
    }

    fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        match &self.host {
            None => Some(range),
            Some(link) => link.segments.copied_span(range),
        }
    }

    fn copies(&self, range: Range<usize>) -> bool {
        self.host
            .as_ref()
            .is_none_or(|link| link.segments.copies(range))
    }
}

/// Whether an embedder wrote a token of `node`, a guest document's, that the host does not spell
/// (Blade's `@unless(c)` `!(`, a `:` after a directive; the parentheses and `;` Vue's view wraps
/// an interpolation in): the node is then not the code its host bytes are, and what reads its
/// tokens (structure, names, references) does not know it. A token copied, or written over the
/// same text (Blade's `@isset(` `isset(`), spells itself. A host document's nodes spell
/// themselves.
pub fn rewritten<D: Doc>(node: &Node<'_, D>) -> bool {
    let Some(doc) = crate::code_rule_dsl::tape::tape_doc(node).filter(|doc| doc.is_guest()) else {
        return false;
    };
    let file = InFile::tape(doc);
    node.dfs()
        .any(|token| !token.is_named() && token.is_leaf() && !file.spells(&token))
}
