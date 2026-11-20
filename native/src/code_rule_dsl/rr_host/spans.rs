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
use std::path::Path;
use std::sync::Arc;

/// A file's tree as a run projects it, and its lines as the run counts them.
pub struct FileSpans {
    facts: Arc<Facts>,
    projected: Projected,
    lines: Lines,
    /// Whether the tree has errors: a run may withhold such a file's tree, and with it its token positions.
    errors: bool,
}

impl FileSpans {
    /// The file at `path` under `root`, parsed as `label`'s dialect (`None`: read as text, as a run reads a file of
    /// no grammar of its own); `None` where it cannot be read.
    pub fn read(root: &Path, path: &str, label: Option<&str>) -> Option<FileSpans> {
        let source = Arc::new(std::fs::read_to_string(root.join(path)).ok()?);
        let lines = Lines::new(&source, label == Some("python"));
        let parsed = label.and_then(|label| parse(label, &source));
        let (facts, dialect) = match parsed {
            Some((facts, dialect)) => (facts, Some(dialect)),
            None => (Arc::new(core::text(Arc::clone(&source))), None),
        };
        let mut projected = Projected::new(0, Arc::clone(&facts), dialect.as_ref(), Shape::Core, false, None);
        // A run that reads `end_point` numbers each node's end after the lines; no other position follows them.
        if let Some(extra) = projected.extra.as_mut() {
            extra.ends = projected.core.len() as u32;
        }
        let errors = !facts.errors().is_empty();
        Some(FileSpans { facts, projected, lines, errors })
    }

    /// The span of the finding at line `line` (1-based; 0 for the file as a whole) whose `at` is the file's node or
    /// position `node` (`doc << 32 | low`), or no node of the file.
    pub fn span(&self, node: Option<i64>, line: i64) -> Option<Span> {
        node.and_then(|value| self.at((value & 0xffff_ffff) as u32, line)).or_else(|| self.line(line))
    }

    /// Node or position `low`'s span, where it starts on line `line`.
    fn at(&self, low: u32, line: i64) -> Option<Span> {
        let p = &self.projected;
        let local = p.local_of(low).filter(|&local| local < p.locs())?;
        if local >= p.len() && self.errors {
            return None;
        }
        let [start, end] = p.span(local);
        (i64::from(self.lines.line(start)) == line).then(|| self.facts.lines.span(start as usize, end as usize))
    }

    /// Line `line`'s text, before its terminator; the file's start for line 0.
    fn line(&self, line: i64) -> Option<Span> {
        let [start, end] = match usize::try_from(line) {
            Ok(0) => [0, 0],
            Ok(l) if l <= self.lines.count() => self.lines.span(l),
            _ => return None,
        };
        Some(self.facts.lines.span(start as usize, end as usize))
    }
}

/// `source` parsed as `label`'s dialect, and the dialect; `None` where it does not parse within the parser's limits.
fn parse(label: &str, source: &Arc<String>) -> Option<(Arc<Facts>, Dialect)> {
    let Ok(Built::Facts(facts)) = core::build(&Source { language: label.to_owned(), source: Arc::clone(source) }) else { return None };
    let dialect = Dialect::new(facts.tag).ok()?;
    Some((Arc::new(facts), dialect))
}
