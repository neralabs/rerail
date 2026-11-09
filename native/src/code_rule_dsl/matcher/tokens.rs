//! `tokens`: tests of anonymous tokens (keywords, operators, punctuation) by text (RER-131).
//!
//! A rule ranges over named nodes: its relations walk named nodes only, so it reads tokens here
//! and in patterns. A named-only document then recognizes a node's tokens on demand
//! ([`crate::code_rule_dsl::tree`]) instead of keeping them.
//!
//! ```json
//! { "kind": "except_clause", "tokens": { "has": "^except$" } }
//! { "kind": "identifier", "tokens": { "before": "^\\.$", "after": "^\\($" } }
//! ```
//!
//! Each key names where a matching token is, from the node:
//!
//! - `has`: one of its own tokens. `below`: a token of it or of any node under it.
//! - `before` / `after`: the child just before / after it, among its parent's children.
//! - `earlier` / `later`: any token of its parent before / after it.
//! - `beforeCode` / `afterCode`: the nearest sibling before / after it that is not a comment.
//!
//! A relation whose target is a bare `regex` reached tokens as well as named nodes before tokens
//! left the walks; the compiler keeps that meaning by adding the matching test
//! (`compile::ast_rule`): `has: {regex: R}` reads as `any: [has R, tokens.has R]`.

use crate::code_rule_dsl::tree::Tree;
use ast_grep_core::Matcher as _;
use regex::Regex;
use std::collections::BTreeSet;

/// Where a token is, from the node a test is asked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Place {
    Own,
    Below,
    Before,
    After,
    Earlier,
    Later,
    /// The nearest sibling before / after the node that is not a comment.
    BeforeCode,
    AfterCode,
    /// The node's child in this field.
    Field(String),
}

impl Place {
    pub(in crate::code_rule_dsl) fn key(&self) -> &str {
        match self {
            Self::Own => "has",
            Self::Below => "below",
            Self::Before => "before",
            Self::After => "after",
            Self::Earlier => "earlier",
            Self::Later => "later",
            Self::BeforeCode => "beforeCode",
            Self::AfterCode => "afterCode",
            Self::Field(_) => "field",
        }
    }

    pub(in crate::code_rule_dsl) fn of_key(key: &str) -> Option<Self> {
        Some(match key {
            "has" => Self::Own,
            "below" => Self::Below,
            "before" => Self::Before,
            "after" => Self::After,
            "earlier" => Self::Earlier,
            "later" => Self::Later,
            "beforeCode" => Self::BeforeCode,
            "afterCode" => Self::AfterCode,
            _ => return None,
        })
    }
}

/// How many parts (named nodes and tokens) a place holds whose text matches: what a `count` over
/// a bare regex counted when its walk reached tokens.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct PartCount {
    /// `Own` (children), `Below` (descendants), `Earlier` / `Later` (siblings).
    pub(in crate::code_rule_dsl) place: Place,
    pub(in crate::code_rule_dsl) regex: Regex,
    pub(in crate::code_rule_dsl) min: usize,
    pub(in crate::code_rule_dsl) max: Option<usize>,
}

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct TokenTest {
    /// Every one holds.
    pub(in crate::code_rule_dsl) tests: Vec<(Place, Regex)>,
    /// Every count is within its bounds.
    pub(in crate::code_rule_dsl) counts: Vec<PartCount>,
    /// `as`: binds the token the first test found (a `has`, `before`, `after`, `beforeCode`,
    /// `afterCode` or field token), for a message, anchor or fix range.
    pub(in crate::code_rule_dsl) capture: Option<String>,
    /// `where`: a rule the first test's token must match too (evaluated at the token, on the host).
    pub(in crate::code_rule_dsl) rule: Option<Box<super::AstMatcher>>,
    /// The kinds the rule's own `kind` names: the only nodes the test is asked at (a fact's
    /// candidates), where it names one.
    pub(in crate::code_rule_dsl) kinds: Option<BTreeSet<u16>>,
}

impl std::fmt::Debug for TokenTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenTest").field("tests", &self.tests).field("counts", &self.counts).field("capture", &self.capture).field("where", &self.rule.is_some()).finish()
    }
}

impl TokenTest {
    pub(in crate::code_rule_dsl) fn match_node<'r, D: ast_grep_core::Doc>(
        &self,
        node: ast_grep_core::Node<'r, D>,
        env: &mut std::borrow::Cow<ast_grep_core::meta_var::MetaVarEnv<'r, D>>,
    ) -> Option<ast_grep_core::Node<'r, D>> {
        let Some((first, rest)) = self.tests.split_first() else {
            return self.holds(&node).then_some(node);
        };
        if !rest.iter().all(|(place, re)| holds(place, re, &node)) || !self.counted(&node) {
            return None;
        }
        if self.capture.is_none() && self.rule.is_none() {
            return holds(&first.0, &first.1, &node).then_some(node);
        }
        // The first token at the place that matches the rule too, bound under `as`.
        for token in found(&first.0, &first.1, &node) {
            let mut trial = env.clone();
            if let Some(rule) = &self.rule
                && rule.match_node_with_env(token.clone(), &mut trial).is_none()
            {
                continue;
            }
            if let Some(name) = &self.capture
                && trial.to_mut().insert(name, token).is_none()
            {
                continue;
            }
            *env = trial;
            return Some(node);
        }
        None
    }

    pub(in crate::code_rule_dsl) fn holds<'r, D: ast_grep_core::Doc>(&self, node: &ast_grep_core::Node<'r, D>) -> bool {
        self.tests.iter().all(|(place, re)| holds(place, re, node)) && self.counted(node)
    }

    fn counted<'r, D: ast_grep_core::Doc>(&self, node: &ast_grep_core::Node<'r, D>) -> bool {
        self.counts.iter().all(|count| {
            let found = parts(count, node);
            found >= count.min && count.max.is_none_or(|max| found <= max)
        })
    }
}

fn parts<'r, D: ast_grep_core::Doc>(count: &PartCount, node: &ast_grep_core::Node<'r, D>) -> usize {
    use crate::code_rule_dsl::tree::Part;
    let text = |part: &Part<'r, D>| match part {
        Part::Named(node) => count.regex.is_match(&node.text()),
        Part::Token(token) => count.regex.is_match(&token.text()),
    };
    match &count.place {
        Place::Own => node.parts().filter(text).count(),
        Place::Below => node.dfs().filter(|n| n.is_named()).map(|n| n.parts().filter(text).count()).sum(),
        Place::Earlier | Place::Later => node.parent().map_or(0, |parent| {
            let span = node.range();
            parent
                .parts()
                .filter(|part| {
                    let at = part.range();
                    if count.place == Place::Earlier { at.end <= span.start } else { at.start >= span.end }
                })
                .filter(text)
                .count()
        }),
        _ => 0,
    }
}

/// The tokens at `place` that match, as nodes, in source order.
fn found<'r, D: ast_grep_core::Doc>(place: &Place, re: &Regex, node: &ast_grep_core::Node<'r, D>) -> Vec<ast_grep_core::Node<'r, D>> {
    let matches = |token: &crate::code_rule_dsl::tree::Token<'r, D>| re.is_match(&token.text());
    let one = match place {
        Place::Own => return node.tokens().filter(matches).map(|token| token.node().clone()).collect(),
        Place::Before => node.token_before().filter(matches).map(|token| token.node().clone()),
        Place::After => node.token_after().filter(matches).map(|token| token.node().clone()),
        Place::BeforeCode | Place::AfterCode => {
            let code = |sibling: &ast_grep_core::Node<'r, D>| !(sibling.is_named() && crate::code_rule_dsl::tree::is_comment(&sibling.kind()));
            let near = if *place == Place::BeforeCode { node.prev_all().find(code) } else { node.next_all().find(code) };
            near.filter(|near| !near.is_named() && re.is_match(&near.text()))
        }
        Place::Field(name) => node.field(name).filter(|child| !child.is_named() && re.is_match(&child.text())),
        Place::Below | Place::Earlier | Place::Later => None,
    };
    one.into_iter().collect()
}

fn holds<'r, D: ast_grep_core::Doc>(place: &Place, re: &Regex, node: &ast_grep_core::Node<'r, D>) -> bool {
    let matches = |token: crate::code_rule_dsl::tree::Token<'r, D>| re.is_match(&token.text());
    match place {
        Place::Own => node.tokens().any(matches),
        Place::Below => node.dfs().filter(|n| n.is_named()).any(|n| n.tokens().any(matches)),
        Place::Before => node.token_before().is_some_and(matches),
        Place::After => node.token_after().is_some_and(matches),
        Place::Earlier | Place::Later => node.parent().is_some_and(|parent| {
            let span = node.range();
            parent.tokens().any(|token| {
                let at = token.range();
                (if *place == Place::Earlier { at.end <= span.start } else { at.start >= span.end }) && matches(token)
            })
        }),
        Place::BeforeCode | Place::AfterCode => {
            let code = |sibling: &ast_grep_core::Node<'r, D>| !(sibling.is_named() && crate::code_rule_dsl::tree::is_comment(&sibling.kind()));
            let near = if *place == Place::BeforeCode { node.prev_all().find(code) } else { node.next_all().find(code) };
            near.is_some_and(|near| !near.is_named() && re.is_match(&near.text()))
        }
        Place::Field(name) => node.field(name).is_some_and(|child| !child.is_named() && re.is_match(&child.text())),
    }
}
