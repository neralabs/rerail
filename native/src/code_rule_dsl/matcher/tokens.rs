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

/// A tape's tokens for the facts stage (`Fact::found`): every named node's tokens recognized once,
/// shared by the tape's `tokens` facts and dropped with the stage. Nothing is kept on the tape.
///
/// Token rows are in the preorder of their owners, so a subtree's tokens are one range
/// (`below`), and a regex is tested once per distinct token text: each test is then a prefix-sum
/// lookup per node.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct TapeTokens {
    index: std::sync::OnceLock<TokenIndex>,
    matched: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Vec<u32>>>>,
}

const NONE: u32 = u32::MAX;

/// A sibling beside a node, among its parent's parts.
#[derive(Clone, Copy)]
enum Near {
    None,
    Token(u32),
    Named,
}

struct Entry {
    own: (u32, u32),
    below_end: u32,
    parent: u32,
    /// How many of its parent's tokens come before it.
    before: u32,
    /// Before, after, before skipping comments, after skipping comments.
    near: [Near; 4],
}

struct TokenIndex {
    entry_of: Vec<u32>,
    entries: Vec<Entry>,
    /// Each row's text, as an index into `texts`.
    rows: Vec<u32>,
    texts: Vec<String>,
}

impl TokenIndex {
    fn build<D: ast_grep_core::Doc>(root: ast_grep_core::Node<'_, D>, len: usize) -> Self {
        use crate::code_rule_dsl::tree::{Part, is_comment};
        let mut index = Self { entry_of: vec![NONE; len], entries: Vec::new(), rows: Vec::new(), texts: Vec::new() };
        let mut texts = std::collections::HashMap::<String, u32>::new();
        let root_id = root.node_id();
        if root_id >= len {
            return index;
        }
        index.entry_of[root_id] = 0;
        index.entries.push(Entry { own: (0, 0), below_end: 0, parent: NONE, before: 0, near: [Near::None; 4] });
        let mut stack = vec![(root, false)];
        while let Some((node, exit)) = stack.pop() {
            let entry = index.entry_of[node.node_id()] as usize;
            if exit {
                index.entries[entry].below_end = index.rows.len() as u32;
                continue;
            }
            let start = index.rows.len() as u32;
            let parts = node.parts().collect::<Vec<_>>();
            let mut row_of = Vec::with_capacity(parts.len());
            for part in &parts {
                row_of.push(match part {
                    Part::Token(token) => {
                        let text = token.text();
                        let next = texts.len() as u32;
                        let id = *texts.entry(text.into_owned()).or_insert(next);
                        index.rows.push(id);
                        Some(index.rows.len() as u32 - 1)
                    }
                    Part::Named(_) => None,
                });
            }
            index.entries[entry].own = (start, index.rows.len() as u32);
            let near = |at: usize| row_of[at].map_or(Near::Named, Near::Token);
            let code = |at: usize| !matches!(&parts[at], Part::Named(named) if is_comment(&named.kind()));
            let mut children = Vec::new();
            let mut tokens_before = 0;
            for (at, part) in parts.iter().enumerate() {
                let Part::Named(child) = part else {
                    tokens_before += 1;
                    continue;
                };
                let id = child.node_id();
                if id >= len || index.entry_of[id] != NONE {
                    continue;
                }
                let before = at.checked_sub(1).map_or(Near::None, near);
                let after = if at + 1 < parts.len() { near(at + 1) } else { Near::None };
                let before_code = (0..at).rev().find(|&i| code(i)).map_or(Near::None, near);
                let after_code = (at + 1..parts.len()).find(|&i| code(i)).map_or(Near::None, near);
                index.entry_of[id] = index.entries.len() as u32;
                index.entries.push(Entry { own: (0, 0), below_end: 0, parent: entry as u32, before: tokens_before, near: [before, after, before_code, after_code] });
                children.push(child.clone());
            }
            stack.push((node, true));
            stack.extend(children.into_iter().rev().map(|child| (child, false)));
        }
        index.texts = {
            let mut ordered = vec![String::new(); texts.len()];
            for (text, id) in texts {
                ordered[id as usize] = text;
            }
            ordered
        };
        index
    }
}

impl TapeTokens {
    fn index<D: ast_grep_core::Doc>(&self, root: ast_grep_core::Node<'_, D>, len: usize) -> &TokenIndex {
        self.index.get_or_init(|| TokenIndex::build(root, len))
    }

    /// The number of matching rows before each row (and in all, last).
    fn matched(&self, index: &TokenIndex, re: &Regex) -> std::sync::Arc<Vec<u32>> {
        let mut matched = self.matched.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::sync::Arc::clone(matched.entry(re.as_str().to_owned()).or_insert_with(|| {
            let texts = index.texts.iter().map(|text| re.is_match(text)).collect::<Vec<_>>();
            let mut prefix = Vec::with_capacity(index.rows.len() + 1);
            let mut sum = 0;
            prefix.push(0);
            for &row in &index.rows {
                sum += u32::from(texts[row as usize]);
                prefix.push(sum);
            }
            std::sync::Arc::new(prefix)
        }))
    }

    /// `test`'s verdicts at `starts`: the nodes it holds at outright, and those the matcher still
    /// asks (an `as`, `where`, count or slot, or a test the index cannot tell: a field token, a
    /// node it does not hold).
    pub(in crate::code_rule_dsl) fn split<D: ast_grep_core::Doc>(
        &self,
        test: &TokenTest,
        plain: bool,
        root: ast_grep_core::Node<'_, D>,
        len: usize,
        starts: impl Iterator<Item = u32>,
    ) -> (Vec<u32>, Vec<u32>) {
        let index = self.index(root, len);
        let tests = test.tests.iter().map(|(place, re)| (place, self.matched(index, re))).collect::<Vec<_>>();
        let (mut held, mut asked) = (Vec::new(), Vec::new());
        'starts: for ordinal in starts {
            let Some(&at) = index.entry_of.get(ordinal as usize).filter(|&&entry| entry != NONE) else {
                asked.push(ordinal);
                continue;
            };
            let entry = &index.entries[at as usize];
            let parent = (entry.parent != NONE).then(|| &index.entries[entry.parent as usize]);
            let mut known = true;
            for (place, prefix) in &tests {
                let any = |(start, end): (u32, u32)| prefix[end as usize] > prefix[start as usize];
                let near = |near: Near| matches!(near, Near::Token(row) if any((row, row + 1)));
                let holds = match place {
                    Place::Own => any(entry.own),
                    Place::Below => any((entry.own.0, entry.below_end)),
                    Place::Before => near(entry.near[0]),
                    Place::After => near(entry.near[1]),
                    Place::BeforeCode => near(entry.near[2]),
                    Place::AfterCode => near(entry.near[3]),
                    Place::Earlier => parent.is_some_and(|parent| any((parent.own.0, parent.own.0 + entry.before))),
                    Place::Later => parent.is_some_and(|parent| any((parent.own.0 + entry.before, parent.own.1))),
                    Place::Field(_) => {
                        known = false;
                        continue;
                    }
                };
                if !holds {
                    continue 'starts;
                }
            }
            if known && plain { held.push(ordinal) } else { asked.push(ordinal) }
        }
        (held, asked)
    }
}

impl TokenTest {
    /// Whether the test is its tests alone: nothing to bind, no rule at the token, no count.
    pub(in crate::code_rule_dsl) fn is_plain(&self) -> bool {
        self.capture.is_none() && self.rule.is_none() && self.counts.is_empty()
    }
}
