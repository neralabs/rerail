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
    /// The nearest token (a leaf of the tree, anonymous ones included).
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
        self.before.is_some() || self.after.is_some()
    }

    pub(super) fn match_node<'t, D: Doc>(
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
            Neighbour::Token => token(node, side, self.comments)?.map(|token| file.node(&token)),
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
/// gap touches, walking from the node toward its neighbour; `None` when the budget ran out.
fn blank_lines<D: Doc>(
    node: &Node<'_, D>,
    side: Side,
    gap: Range<usize>,
    file: &InFile<'_>,
) -> Option<usize> {
    let lines = file.lines();
    let (first, last) = (lines.line(gap.start), lines.line(gap.end));
    let mut blank = 0;
    // The line the walk last left: the node's first (before) or last (after).
    let mut at = match side {
        Side::Before => last,
        Side::After => first,
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
        let (near, far) = match side {
            Side::Before => (lines.line(token.end), lines.line(token.start)),
            Side::After => (lines.line(token.start), lines.line(token.end)),
        };
        blank += near.abs_diff(at).saturating_sub(1);
        at = far;
        current = next;
    }
    let end = match side {
        Side::Before => first,
        Side::After => last,
    };
    Some(blank + end.abs_diff(at).saturating_sub(1))
}

/// Whether a node is a comment, which a token or sibling walk passes over unless asked not to.
fn is_comment<D: Doc>(node: &Node<'_, D>) -> bool {
    matches!(node.kind().as_ref(), "comment" | "html_comment")
}

/// The nearest token on `side` of the node that has a width, comments included or passed over;
/// `None` when the budget ran out.
fn token<'t, D: Doc>(
    node: &Node<'t, D>,
    side: Side,
    comments: bool,
) -> Option<Option<Node<'t, D>>> {
    let mut current = node.clone();
    loop {
        if !spend_util_step() {
            return None;
        }
        let Some(next) = leaf(&current, side) else {
            return Some(None);
        };
        if comments || !is_comment(&next) {
            return Some(Some(next));
        }
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
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn python(rule: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        compile_document_with(
            clause_document(json!({ "id": "c", "violation": violation(),
                "forbid": { "ast": { "language": "python", "rule": rule } } })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason))
    }

    #[test]
    fn span_counts_the_line_breaks_inside_the_node() {
        let one_line = compiled(
            json!({ "rule": { "kind": "if_statement", "layout": { "span": { "eq": 0 } } } }),
        );
        let source = "if (a) b();\nif (c)\n  d();\nif (e) {\n}\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&one_line, "typescript", "a.ts", source),
                source
            ),
            ["if (a) b();"]
        );
        let two = compiled(
            json!({ "rule": { "kind": "if_statement", "layout": { "span": { "min": 1, "max": 1 } } } }),
        );
        assert_eq!(
            matched_texts(&evaluate_file(&two, "typescript", "a.ts", source), source),
            ["if (c)\n  d();", "if (e) {\n}"]
        );
        // ESLint breaks lines at CR, CRLF, U+2028 and U+2029 too; Ruff not at the last two.
        let string =
            compiled(json!({ "rule": { "kind": "string", "layout": { "span": { "eq": 1 } } } }));
        let source = "x = 'a\u{2028}b';\ny = `c\r\nd`;\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&string, "typescript", "a.ts", source),
                source
            ),
            ["'a\u{2028}b'"]
        );
        let string = python(json!({ "kind": "string", "layout": { "span": { "min": 1 } } }));
        let source = "x = 'a\u{2028}b'\ny = '''c\rd'''\n";
        assert_eq!(
            matched_texts(&evaluate_file(&string, "python", "a.py", source), source),
            ["'''c\rd'''"]
        );
    }

    #[test]
    fn start_reads_the_node_s_first_byte() {
        // Ruff's file-level diagnostics sit at byte 0, a shebang's too.
        let first = python(json!({ "kind": "comment", "layout": { "start": { "eq": 0 } } }));
        let source = "#!/usr/bin/env python\nimport os  # a comment\n";
        assert_eq!(
            matched_texts(&evaluate_file(&first, "python", "a.py", source), source),
            ["#!/usr/bin/env python"]
        );
        let later = python(
            json!({ "kind": "identifier", "layout": { "start": { "min": 30, "max": 40 } } }),
        );
        assert_eq!(
            matched_texts(&evaluate_file(&later, "python", "a.py", source), source),
            Vec::<String>::new()
        );
        let source = "import os\n";
        assert_eq!(
            matched_texts(&evaluate_file(&first, "python", "a.py", source), source),
            Vec::<String>::new()
        );
        let statement =
            python(json!({ "kind": "import_statement", "layout": { "start": { "eq": 0 } } }));
        assert_eq!(
            matched_texts(&evaluate_file(&statement, "python", "a.py", source), source),
            ["import os"]
        );
    }

    #[test]
    fn a_missing_neighbour_fails_the_gap_unless_the_file_s_edge_stands_in() {
        let gap = |side: &str, edge: bool, breaks: u64| {
            compiled(json!({ "rule": { "kind": "comment", "layout": {
                side: { "to": "token", "edge": edge, "breaks": { "eq": breaks } } } } }))
        };
        let source = "\n// head\nf(); // tail\n";
        let texts = |compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl| {
            matched_texts(
                &evaluate_file(compiled, "typescript", "a.ts", source),
                source,
            )
        };
        assert_eq!(texts(&gap("before", false, 1)), Vec::<String>::new());
        assert_eq!(texts(&gap("before", true, 1)), ["// head"]);
        assert_eq!(texts(&gap("after", false, 1)), ["// head"]);
        assert_eq!(texts(&gap("after", true, 1)), ["// head", "// tail"]);
        // The file's start stands in as a token on the first line.
        let blank = compiled(json!({ "rule": { "kind": "comment", "layout": {
            "before": { "to": "token", "edge": true, "blank": { "eq": 0 } } } } }));
        assert_eq!(texts(&blank), ["// head", "// tail"]);
    }

    #[test]
    fn comments_count_as_neighbours_only_when_asked() {
        let sibling = |comments: bool| {
            compiled(json!({ "rule": { "kind": "identifier", "regex": "^b$",
                "layout": { "before": { "to": "sibling", "comments": comments, "breaks": { "eq": 1 } } } } }))
        };
        let source = "f(a,\n  /* c */ b);\ng(a, /* c */\n  b);\n";
        let texts = |compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl| {
            matched_texts(
                &evaluate_file(compiled, "typescript", "a.ts", source),
                source,
            )
        };
        assert_eq!(texts(&sibling(false)), ["b", "b"]);
        assert_eq!(texts(&sibling(true)), ["b"]);
        // A capture must lie wholly on its side of the node.
        let capture = compiled(json!({ "rule": { "kind": "identifier", "regex": "^a$",
            "inside": { "kind": "call_expression", "pattern": "$C" },
            "layout": { "before": { "to": "capture.C", "breaks": { "min": 0 } } } } }));
        assert_eq!(texts(&capture), Vec::<String>::new());
    }

    #[test]
    fn a_negated_gap_is_unsound_under_syntax_errors() {
        let source = "f(a, b\nlet = ;\n";
        let failures = |rule: Value| {
            let result = evaluate_file(
                &compiled(json!({ "rule": rule })),
                "typescript",
                "a.ts",
                source,
            );
            result["failures"]
                .as_array()
                .expect("failures")
                .iter()
                .map(|failure| failure["code"].as_str().expect("code").to_owned())
                .collect::<Vec<_>>()
        };
        let gap = json!({ "before": { "to": "token", "breaks": { "eq": 0 } } });
        assert_eq!(
            failures(json!({ "kind": "identifier", "layout": gap })),
            ["source-partial"]
        );
        assert_eq!(
            failures(json!({ "kind": "identifier", "not": { "layout": gap } })),
            ["source-incomplete"]
        );
        assert_eq!(
            failures(json!({ "kind": "identifier", "not": { "layout": { "span": { "eq": 0 } } } })),
            ["source-partial"]
        );
    }
}
