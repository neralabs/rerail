//! `inline-content`: a Markdown file's inline content, each `inline` and `pipe_table_cell` node a program of its own
//! in the inline grammar (decision 0238; 0431 §4). tree-sitter-md reads a Markdown file in two grammars: the block
//! grammar gives the file's tree (headings, lists, paragraphs, code blocks), and each such node holds text in the
//! inline grammar (`markdown-inline`: emphasis, links, code spans, inline HTML).
//!
//! ```text
//! > Some *text* and a [link](u)       block: block_quote > paragraph > inline
//! > on two lines.                     inline program: emphasis, inline_link > link_text, link_destination
//! ```
//!
//! A program's text is the node's without its `block_continuation`s after the first child (the `> ` on a continued
//! line above), as tree-sitter-md's `MarkdownParser` gives its inline parser: each piece a copied stretch of the file,
//! so its nodes have the file's lines and columns, and its host node is the `inline` or cell (the innermost host
//! node over the bytes its map names).

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;

/// The embedder of `Via::InlineContent`.
pub struct InlineContent;

impl Embedder for InlineContent {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let mut programs = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if !matches!(node.kind(), "inline" | "pipe_table_cell") {
                let children = node.children().collect::<Vec<_>>();
                stack.extend(children.into_iter().rev());
                continue;
            }
            let [start, end] = [node.start_byte(), node.end_byte()];
            let mut text = String::new();
            let mut segments = SegmentMap::new();
            let mut from = start;
            let mut copy = |text: &mut String, range: std::ops::Range<usize>| {
                if range.is_empty() {
                    return;
                }
                let at = text.len();
                text.push_str(&source[range.clone()]);
                segments.push(at, text.len(), range, true, false);
            };
            // tree-sitter-md leaves the first child in (its loop starts at the second), as here.
            for c in node.children().skip(1).filter(|c| c.kind() == "block_continuation") {
                copy(&mut text, from..c.start_byte());
                from = c.end_byte();
            }
            copy(&mut text, from..end);
            if text.is_empty() {
                continue;
            }
            segments.index();
            let dialect = GrammarDialect::MarkdownInline;
            let Some(parse) = super::Parse::read_program(dialect, &text, &segments, budget) else { continue };
            programs.push(Program { dialect, parse, text, segments, host_errors: Vec::new(), at: None });
        }
        programs
    }

    fn per_place(&self) -> bool {
        true
    }
}
