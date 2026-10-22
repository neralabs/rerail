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
pub(crate) trait Origin {
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
pub(crate) struct InFile<'t> {
    source: &'t str,
    dialect: GrammarDialect,
    lines: Lines<'t>,
    origin: &'t dyn Origin,
}

impl<'t> InFile<'t> {
    /// The file the document of `node` is read from.
    pub(crate) fn of<D: Doc>(node: &Node<'t, D>) -> Option<Self> {
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
                Terminators::of(dialect),
            )),
            origin: &Same,
        })
    }

    /// The file a tape document is read from: its own, or for a guest its host's.
    pub(crate) fn tape(doc: &'t TapeDoc) -> Self {
        Self {
            source: doc.file_source().as_str(),
            dialect: doc.file_dialect(),
            lines: Lines::Taped(doc.file_lines()),
            origin: doc,
        }
    }

    /// The file's whole text.
    pub(crate) fn source(&self) -> &'t str {
        self.source
    }

    /// The file's dialect.
    pub(crate) fn dialect(&self) -> GrammarDialect {
        self.dialect
    }

    /// The file's lines.
    pub(crate) fn lines(&self) -> &LineTable {
        match &self.lines {
            Lines::Taped(lines) => lines,
            Lines::Read(lines) => lines,
        }
    }

    /// The file ranges a node of the document stands for.
    pub(crate) fn place<D: Doc>(&self, node: &Node<'_, D>) -> FileRanges {
        self.origin
            .of_node(node.node_id(), node.range())
            .expect("a document's node lies in its file")
    }

    /// The span a node reports: its primary file range (`FileRanges::primary`).
    pub(crate) fn node<D: Doc>(&self, node: &Node<'_, D>) -> Range<usize> {
        self.place(node).primary()
    }

    /// The file ranges a range of the document stands for.
    pub(crate) fn places(&self, range: Range<usize>) -> FileRanges {
        self.origin
            .of_range(range)
            .expect("a document's range lies in its file")
    }

    /// The span a range of the document reports: its primary file range.
    pub(crate) fn range(&self, range: Range<usize>) -> Range<usize> {
        self.places(range).primary()
    }

    /// The file bytes a range of the document copies, where it does.
    pub(crate) fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
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
    pub(crate) fn text<D: Doc>(&self, node: &Node<'t, D>) -> Cow<'t, str> {
        match self
            .place(node)
            .single()
            .and_then(|range| self.source.get(range))
        {
            Some(text) => Cow::Borrowed(text),
            None => node.text(),
        }
    }

    /// The text of a range of the document, as `text` reads a node's: the file's where it stands
    /// for one range, else `own`.
    pub(crate) fn range_text<'a>(&self, range: Range<usize>, own: &'a str) -> Cow<'a, str>
    where
        't: 'a,
    {
        match self
            .places(range)
            .single()
            .and_then(|range| self.source.get(range))
        {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Borrowed(own),
        }
    }

    /// The diagnostic span of file bytes `range`.
    pub(crate) fn span(&self, range: Range<usize>) -> Span {
        self.lines().span(range.start, range.end)
    }
}
