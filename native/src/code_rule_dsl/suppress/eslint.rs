//! ESLint's `eslint-disable` comments: `suppress.eslint`, and the file's directives that honour it.

use super::{names, root};
use crate::code_rule_dsl::json::{object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use crate::code_rule_dsl::literal::eslint_directive::Directive;
use ast_grep_core::{Doc, Node};
use napi::Result;
use serde_json::Value;

/// `eslint.rules`: the rule names an `eslint-disable` comment must name (or name none) to
/// suppress the violation.
pub(super) fn spec(eslint: &Value, at: &str) -> Result<Vec<String>> {
    reject_unknown(object(eslint, at)?, &["rules"], at)?;
    names(
        eslint.get("rules"),
        &format!("{at}/rules"),
        rule_name,
        "rule names",
        "a rule name (non-empty, without commas or surrounding white space)",
    )
}

/// A rule name an `eslint-disable` list can name: ESLint splits the list on commas and trims.
fn rule_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(',') && name.trim() == name
}

/// A block directive: `eslint-disable` or `eslint-enable` where its comment starts.
#[derive(Debug)]
struct Block {
    start: usize,
    disable: bool,
    /// The rule ids it names; none names every rule.
    rules: Vec<String>,
}

/// A file's ESLint directives that disable or enable rules, as ESLint 10.11.0's
/// `getDisableDirectives` and `applyDisableDirectives` read them.
#[derive(Debug, Default)]
pub(crate) struct EslintDirectives {
    /// Block directives, in source order.
    blocks: Vec<Block>,
    /// Line directives: the line each applies to, and the rule ids it names.
    lines: Vec<(usize, Vec<String>)>,
}

impl EslintDirectives {
    /// Reads every comment of the file `node` belongs to, HTML-like ones (`html_comment`, which
    /// ESLint reads as line comments) included.
    pub(crate) fn read<D: Doc>(node: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        Self::from_comments(
            root(node)
                .dfs()
                .filter(|node| matches!(node.kind().as_ref(), "comment" | "html_comment"))
                .map(|comment| comment.range()),
            source,
            lines,
        )
    }

    /// Reads the comments at `comments`, ranges of `source`: the comments ESLint's source code
    /// holds.
    pub(crate) fn from_comments(
        comments: impl Iterator<Item = std::ops::Range<usize>>,
        source: &str,
        lines: &LineTable,
    ) -> Self {
        let mut directives = Self::default();
        for range in comments {
            let text = &source[range.clone()];
            let Some(directive) = Directive::parse(text).filter(Directive::disables) else {
                continue;
            };
            let rules = directive
                .items(text)
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>();
            let (first, last) = (lines.line(range.start), lines.line(range.end));
            match directive.label.as_str() {
                "eslint-disable" | "eslint-enable" => directives.blocks.push(Block {
                    start: range.start,
                    disable: directive.label == "eslint-disable",
                    rules,
                }),
                // A `-line` directive over several lines is a problem, not a directive.
                "eslint-disable-line" if first == last => directives.lines.push((first, rules)),
                "eslint-disable-next-line" => directives.lines.push((last + 1, rules)),
                _ => {}
            }
        }
        directives
    }

    /// Whether a problem of a rule named `rules` (any of its names) starting at byte `at` is
    /// suppressed: the last block directive at or before it that names the rule (or none) is a
    /// disable, or a line directive naming it applies to its line.
    pub(crate) fn suppresses(&self, rules: &[String], at: usize, lines: &LineTable) -> bool {
        let names = |ids: &[String]| ids.is_empty() || ids.iter().any(|id| rules.contains(id));
        if self
            .blocks
            .iter()
            .rev()
            .find(|block| block.start <= at && names(&block.rules))
            .is_some_and(|block| block.disable)
        {
            return true;
        }
        let line = lines.line(at);
        self.lines
            .iter()
            .any(|(target, ids)| *target == line && names(ids))
    }
}
