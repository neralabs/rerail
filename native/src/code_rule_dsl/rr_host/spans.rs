//! A finding's span, read after the run from the file it is in: the byte range of the node or position its `at` names,
//! and their lines and byte columns as the JSON path gives a violation's (`lines::LineTable`, 0-based).
//!
//! A run numbers a file's nodes and positions in its projection (`syntax::Projected`, `core.rerail`'s shape, every
//! node: a policy's run is never narrowed by anchors, which only a phase whose exports hold no node is). The file is
//! parsed again here and projected the same way, so a node id's low bits name the same node. Nothing is added to the
//! run itself. Where that cannot name the place exactly (a `@place`, a comment or match span, a guest's node, a file
//! whose tree has errors and whose positions a run may have withheld), the span is the finding's line's text; a
//! node whose start is not on the finding's line is not taken either.

use crate::code_rule_dsl::facts::core::{self, Built, Facts, Source};
use crate::code_rule_dsl::facts::vtab::syntax::core_rr::Lines;
use crate::code_rule_dsl::facts::vtab::syntax::{Dialect, Projected, Shape};
use crate::code_rule_dsl::lines::Span;
use crate::code_rule_dsl::tape::TapeDoc;
use std::path::Path;
use std::sync::Arc;

/// A file's tree as a run projects it, with its lines as the run counts them (the projection's own).
pub struct FileSpans {
    facts: Arc<Facts>,
    projected: Projected,
    /// Whether the tree has errors: a run may withhold such a file's tree, and with it its token positions.
    errors: bool,
    /// The file's tree as the matchers read it (`read_with_tree`), from the same parse.
    tree: Option<TapeDoc>,
}

impl FileSpans {
    /// The file at `path` under `root`, parsed as `label`'s dialect (`None`: read as text, as a run reads a file of
    /// no grammar of its own); `None` where it cannot be read.
    pub fn read(root: &Path, path: &str, label: Option<&str>) -> Option<FileSpans> {
        Self::read_as(root, path, label, false)
    }

    /// `read`, keeping the file's tree as the matchers read it (`tree`): its document's own, no guest's.
    pub fn read_with_tree(root: &Path, path: &str, label: Option<&str>) -> Option<FileSpans> {
        Self::read_as(root, path, label, true)
    }

    fn read_as(root: &Path, path: &str, label: Option<&str>, keep: bool) -> Option<FileSpans> {
        let source = Arc::new(std::fs::read_to_string(root.join(path)).ok()?);
        let parsed = label.and_then(|label| parse(label, &source, keep));
        let (facts, dialect, tree) = match parsed {
            Some((facts, dialect, tree)) => (facts, Some(dialect), tree),
            None => (Arc::new(core::text(Arc::clone(&source))), None, None),
        };
        let mut projected = Projected::new(0, Arc::clone(&facts), dialect.as_ref(), Shape::Core, false, None);
        // A run that reads `end_point` numbers each node's end after the lines; no other position follows them.
        if let Some(extra) = projected.extra.as_mut() {
            extra.ends = projected.core.len() as u32;
        }
        let errors = !facts.errors().is_empty();
        Some(FileSpans { facts, projected, errors, tree })
    }

    /// The file's tree as the matchers read it, where `read_with_tree` read one (a file of no grammar has none).
    pub fn tree(&self) -> Option<&TapeDoc> {
        self.tree.as_ref()
    }

    /// The span of the finding at line `line` (1-based; 0 for the file as a whole) whose `at` is the file's node or
    /// position `node` (`doc << 32 | low`), or no node of the file.
    pub fn span(&self, node: Option<i64>, line: i64) -> Option<Span> {
        node.and_then(|value| self.at((value & 0xffff_ffff) as u32, line)).or_else(|| self.line(line))
    }

    /// The span of the file's node or position `node` (`doc << 32 | low`), wherever it starts; `None` where the
    /// projection cannot name it exactly.
    pub fn node_span(&self, node: i64) -> Option<Span> {
        let [start, end] = self.located((node & 0xffff_ffff) as u32)?;
        Some(self.facts.lines.span(start as usize, end as usize))
    }

    /// The file's lines as the run counts them: the projection's (`Shape::Core` always has them).
    fn lines(&self) -> Option<&Lines> {
        self.projected.extra.as_ref().map(|extra| extra.lines())
    }

    /// Node or position `low`'s span, where it starts on line `line`.
    fn at(&self, low: u32, line: i64) -> Option<Span> {
        let [start, end] = self.located(low)?;
        (i64::from(self.lines()?.line(start)) == line).then(|| self.facts.lines.span(start as usize, end as usize))
    }

    /// Node or position `low`'s bytes, where the projection names them.
    fn located(&self, low: u32) -> Option<[u32; 2]> {
        let p = &self.projected;
        let local = p.local_of(low).filter(|&local| local < p.locs())?;
        if local >= p.len() && self.errors {
            return None;
        }
        Some(p.span(local))
    }

    /// Line `line`'s text, before its terminator; the file's start for line 0.
    fn line(&self, line: i64) -> Option<Span> {
        let lines = self.lines()?;
        let [start, end] = match usize::try_from(line) {
            Ok(0) => [0, 0],
            Ok(l) if l <= lines.count() => lines.span(l),
            _ => return None,
        };
        Some(self.facts.lines.span(start as usize, end as usize))
    }
}

/// `source` parsed as `label`'s dialect, the dialect, and with `keep` the tree the matchers read; `None` where it does
/// not parse within the parser's limits.
fn parse(label: &str, source: &Arc<String>, keep: bool) -> Option<(Arc<Facts>, Dialect, Option<TapeDoc>)> {
    let Ok((Built::Facts(facts), tree)) = core::build_with_tape(&Source { language: label.to_owned(), source: Arc::clone(source) }, keep) else { return None };
    let dialect = Dialect::new(facts.tag).ok()?;
    Some((Arc::new(facts), dialect, tree))
}
