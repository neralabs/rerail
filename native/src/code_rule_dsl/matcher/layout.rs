//! `layout` (a Rerail extension): the node's lines and the lines between it and a neighbour, read
//! with the line terminators of the linter a port follows (ESLint's for TypeScript, Ruff's for
//! Python).

use super::each::binding_range;
use super::spend_util_step;
use crate::code_rule_dsl::origin::InFile;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::ops::Range;

/// A count's bounds, inclusive; `eq` is `min == max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineCount {
    pub(crate) min: u64,
    pub(crate) max: Option<u64>,
}

impl LineCount {
    pub(crate) fn holds(self, count: u64) -> bool {
        count >= self.min && self.max.is_none_or(|max| count <= max)
    }
}

/// The node a gap runs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Neighbour {
    /// The nearest token: a leaf of the tree, anonymous ones included, or a token the grammar
    /// implies, which a tape does not store (see [`token`]).
    Token,
    /// The nearest named sibling.
    Sibling,
    /// A single-node capture the rule bound before.
    Capture(String),
}

/// `before` or `after`: the lines between the node and its neighbour on that side.
#[derive(Clone, Debug)]
pub(crate) struct Gap {
    pub(crate) to: Neighbour,
    /// Whether a comment counts as the neighbouring token or sibling (else it is passed over).
    pub(crate) comments: bool,
    /// Whether the file's start (`before`) or end (`after`) stands in for a missing neighbour.
    pub(crate) edge: bool,
    /// Line breaks between the neighbour and the node.
    pub(crate) breaks: Option<LineCount>,
    /// Lines between them that hold no token or comment.
    pub(crate) blank: Option<LineCount>,
}

#[derive(Clone, Debug)]
pub(crate) struct Layout {
    /// Line breaks inside the node.
    pub(crate) span: Option<LineCount>,
    /// The node's first byte.
    pub(crate) start: Option<LineCount>,
    pub(crate) before: Option<Gap>,
    pub(crate) after: Option<Gap>,
    pub(crate) line: Option<regex::Regex>,
    pub(crate) previous_line: Option<regex::Regex>,
    pub(crate) context: Option<LineContext>,
}

#[derive(Clone, Debug)]
pub(crate) struct LineContext {
    pub(crate) before: usize,
    pub(crate) after: usize,
    pub(crate) through: Option<regex::Regex>,
    pub(crate) regex: regex::Regex,
}

impl Layout {
    /// The captures the gaps read.
    pub(crate) fn reads(&self) -> impl Iterator<Item = &str> {
        [&self.before, &self.after]
            .into_iter()
            .flatten()
            .filter_map(|gap| match &gap.to {
                Neighbour::Capture(name) => Some(name.as_str()),
                _ => None,
            })
    }

    /// Whether a gap reads outside the node: its neighbour, and the lines up to it.
    pub(crate) fn outward(&self) -> bool {
        self.before.is_some()
            || self.after.is_some()
            || self.line.is_some()
            || self.previous_line.is_some()
            || self.context.is_some()
    }

    pub(crate) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        // Positions and lines are the file's (`origin`): an embedded program's node lies where the
        // bytes it stands for do. A parsed tree's nodes have no lines of the file.
        let file = InFile::of(&node);
        let range = file
            .as_ref()
            .map_or_else(|| node.range(), |file| file.node(&node));
        if self
            .start
            .is_some_and(|start| !start.holds(range.start as u64))
        {
            return None;
        }
        if self.span.is_none() && !self.outward() {
            return Some(node);
        }
        let file = file?;
        let lines = file.lines();
        let source = file.source();
        let at = lines.line(range.start);
        if self
            .line
            .as_ref()
            .is_some_and(|regex| !regex.is_match(&source[lines.start(at)..lines.end(at)]))
        {
            return None;
        }
        if let Some(regex) = &self.previous_line
            && (at == 0 || !regex.is_match(&source[lines.start(at - 1)..lines.end(at - 1)]))
        {
            return None;
        }
        if let Some(context) = &self.context {
            let first = at.saturating_sub(context.before);
            let mut last = (at + context.after).min(lines.line(source.len()));
            if let Some(through) = &context.through {
                for line in first..=last {
                    if !spend_util_step() {
                        return None;
                    }
                    if through.is_match(&source[lines.start(line)..lines.end(line)]) {
                        last = line;
                        break;
                    }
                }
            }
            if !context
                .regex
                .is_match(&source[lines.start(first)..lines.end(last)])
            {
                return None;
            }
        }

        if let Some(span) = self.span {
            let breaks = lines.line(range.end) - lines.line(range.start);
            if !span.holds(breaks as u64) {
                return None;
            }
        }
        for (gap, side) in [(&self.before, Side::Before), (&self.after, Side::After)] {
            if let Some(gap) = gap
                && !gap.holds(&node, side, env, &file)?
            {
                return None;
            }
        }
        Some(node)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

impl Gap {
    /// Whether the lines between the node and its neighbour on `side` count as the gap asks;
    /// `None` when the file's utility budget ran out.
    fn holds<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        side: Side,
        env: &MetaVarEnv<'t, D>,
        file: &InFile<'_>,
    ) -> Option<bool> {
        let lines = file.lines();
        let range = file.node(node);
        let neighbour = match &self.to {
            Neighbour::Token => token(node, side, self.comments, file)?,
            Neighbour::Sibling => {
                sibling(node, side, self.comments).map(|sibling| file.node(&sibling))
            }
            // The capture must lie wholly on its side of the node.
            Neighbour::Capture(name) => binding_range(env, name)
                .map(|capture| file.range(capture))
                .filter(|capture| match side {
                    Side::Before => capture.end <= range.start,
                    Side::After => capture.start >= range.end,
                }),
        };
        let length = file.source().len();
        let edge = match side {
            Side::Before => 0..0,
            Side::After => length..length,
        };
        let Some(neighbour) = neighbour.or_else(|| self.edge.then_some(edge)) else {
            return Some(false);
        };
        // The lines the gap runs between: the neighbour's end and the node's start, or the node's
        // end and the neighbour's start.
        let (from, to) = match side {
            Side::Before => (neighbour.end, range.start),
            Side::After => (range.end, neighbour.start),
        };
        let (first, last) = (lines.line(from), lines.line(to));
        if self
            .breaks
            .is_some_and(|breaks| !breaks.holds(last.saturating_sub(first) as u64))
        {
            return Some(false);
        }
        let Some(blank) = self.blank else {
            return Some(true);
        };
        let blank_lines = if last <= first + 1 {
            0
        } else {
            blank_lines(node, side, from..to, file)?
        };
        Some(blank.holds(blank_lines as u64))
    }
}

/// The lines strictly between `gap`'s first and last lines that no token or comment inside the
/// gap touches, walking from the node toward its neighbour; `None` when the budget ran out. A line
/// between two stored leaves is blank when its text is all space: an implied token there is the
/// only text a tape does not store (see [`token`]).
fn blank_lines<D: Doc>(
    node: &Node<'_, D>,
    side: Side,
    gap: Range<usize>,
    file: &InFile<'_>,
) -> Option<usize> {
    let lines = file.lines();
    let source = file.source();
    // The lines strictly inside `stretch` (between two leaves) whose text is all space.
    let blank_in = |stretch: Range<usize>| {
        let (first, last) = (lines.line(stretch.start), lines.line(stretch.end));
        (first + 1..last).filter(|&line| source.get(lines.start(line)..lines.end(line)).is_some_and(|text| text.chars().all(is_space))).count()
    };
    let mut blank = 0;
    // Where the walk last left: the node's start (before) or end (after), then each leaf's far
    // edge.
    let mut at = match side {
        Side::Before => gap.end,
        Side::After => gap.start,
    };
    let mut current = node.clone();
    loop {
        if !spend_util_step() {
            return None;
        }
        let Some(next) = leaf(&current, side) else {
            break;
        };
        let token = file.node(&next);
        let inside = match side {
            Side::Before => token.end > gap.start,
            Side::After => token.start < gap.end,
        };
        if !inside {
            break;
        }
        blank += blank_in(match side {
            Side::Before => token.end..at,
            Side::After => at..token.start,
        });
        at = match side {
            Side::Before => token.start,
            Side::After => token.end,
        };
        current = next;
    }
    Some(blank + blank_in(match side {
        Side::Before => gap.start..at,
        Side::After => at..gap.end,
    }))
}

/// Space between tokens: what the grammars' extras pass over (Unicode white space, and the
/// byte-order mark and zero-width characters tree-sitter-python's extras name).
fn is_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{feff}' | '\u{2060}' | '\u{200b}')
}

/// The token in `stretch` nearest the node: `stretch` runs between the node and the next stored
/// leaf on `side`, so any text in it that is not space is a token the grammar implies and the tape
/// leaves out (comments and extras are stored). Its first byte (after) or its last (before):
/// what the gap's lines are counted to. The read is that bounded window, nothing more (t31).
fn implied_in(source: &str, stretch: Range<usize>, side: Side) -> Option<Range<usize>> {
    let text = source.get(stretch.clone())?;
    match side {
        Side::After => {
            let at = stretch.start + (text.len() - text.trim_start_matches(is_space).len());
            (at < stretch.end).then(|| at..at + 1)
        }
        Side::Before => {
            let at = stretch.start + text.trim_end_matches(is_space).len();
            (at > stretch.start).then(|| at - 1..at)
        }
    }
}

/// Whether a node is a comment, which a token or sibling walk passes over unless asked not to.
fn is_comment<D: Doc>(node: &Node<'_, D>) -> bool {
    super::util::is_comment_kind(&node.kind())
}

/// The span of the nearest token on `side` of the node that has a width, comments included or
/// passed over; `None` when the budget ran out. Between the node and each stored leaf the walk
/// meets, a token the grammar implies may sit ([`implied_in`]); the nearest of those, or of the
/// leaves, is the neighbour. Past the document's last leaf, its root's edge bounds the window.
fn token<'t, D: Doc>(
    node: &Node<'t, D>,
    side: Side,
    comments: bool,
    file: &InFile<'_>,
) -> Option<Option<Range<usize>>> {
    let source = file.source();
    let mut current = node.clone();
    let mut near = file.node(node);
    loop {
        if !spend_util_step() {
            return None;
        }
        let next = leaf(&current, side);
        let far = match &next {
            Some(leaf) => file.node(leaf),
            None => file.node(&node.ancestors().last().unwrap_or_else(|| node.clone())),
        };
        let stretch = match (side, next.is_some()) {
            (Side::After, true) => near.end..far.start,
            (Side::After, false) => near.end..far.end,
            (Side::Before, true) => far.end..near.start,
            (Side::Before, false) => far.start..near.start,
        };
        if let Some(implied) = implied_in(source, stretch, side) {
            return Some(Some(implied));
        }
        let Some(next) = next else {
            return Some(None);
        };
        if comments || !is_comment(&next) {
            return Some(Some(far));
        }
        near = far;
        current = next;
    }
}

/// The next leaf with a width on `side` of the node, in document order.
fn leaf<'t, D: Doc>(node: &Node<'t, D>, side: Side) -> Option<Node<'t, D>> {
    let mut current = node.clone();
    loop {
        let mut next = loop {
            let sibling = match side {
                Side::Before => current.prev(),
                Side::After => current.next(),
            };
            match sibling {
                Some(sibling) => break sibling,
                None => current = current.parent()?,
            }
        };
        // Descend to the leaf nearest the node.
        loop {
            let child = match side {
                Side::Before => next.children().last(),
                Side::After => next.children().next(),
            };
            match child {
                Some(child) => next = child,
                None => break,
            }
        }
        if !next.range().is_empty() {
            return Some(next);
        }
        current = next;
    }
}

/// The nearest named sibling on `side` of the node, comments included or passed over.
fn sibling<'t, D: Doc>(node: &Node<'t, D>, side: Side, comments: bool) -> Option<Node<'t, D>> {
    let wanted = |sibling: &Node<'t, D>| sibling.is_named() && (comments || !is_comment(sibling));
    match side {
        Side::Before => node.prev_all().find(wanted),
        Side::After => node.next_all().find(wanted),
    }
}

#[cfg(test)]
mod tests;
