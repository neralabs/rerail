//! The gap test (t31): a relation to a bare token regex, where the token is one the tape leaves
//! out because the grammar implies it.
//!
//! A tape stores no token its grammar implies (RER-131): `f(a, b)` holds `f`, `a` and `b`, not the
//! parentheses or the comma. A rule that asks "is the node before me a `,`" can no longer find the
//! `,` as a node. It asks the grammar instead: the parent's stored children, walked through its
//! automaton, say which word each gap between them holds ([`TokenGrammar::gaps`]). That reads the
//! kinds and fields of one parent's children and static tables, never the source; nothing is
//! cached or indexed.
//!
//! ```text
//! argument_list  f(a, b)       stored: a, b       gaps: before a "(", before b ",", end ")"
//! follows: { regex: "^,$" }    on b: the word before b ends in ","  -> holds
//! ```
//!
//! The compiler adds the test beside the relation's own walk (`compile::ast_rule`), so a stored
//! token still matches as a node and an implied one through its gap.

use crate::code_rule_dsl::tape::tokens::{Gaps, TokenGrammar};
use ast_grep_core::{Doc, Node};
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

/// Where, relative to the node, the token is looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Place {
    /// `has` (a neighbour): any of the node's own tokens.
    Own,
    /// `has` with a `field`: the node's tokens in that field.
    Field(String),
    /// `follows` (a neighbour): the token right before the node.
    Before,
    /// `precedes` (a neighbour): the token right after it.
    After,
    /// `follows` to the end: any token before it under its parent.
    Earlier,
    /// `precedes` to the end: any token after it under its parent.
    Later,
}

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct GapTest {
    pub(in crate::code_rule_dsl) place: Place,
    pub(in crate::code_rule_dsl) regex: Regex,
    pub(in crate::code_rule_dsl) grammar: Arc<TokenGrammar>,
}

impl std::fmt::Debug for GapTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GapTest").field("place", &self.place).field("regex", &self.regex.as_str()).finish()
    }
}

impl GapTest {
    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let matches = |token: &(&str, &str)| self.regex.is_match(token.0);
        match &self.place {
            Place::Own | Place::Field(_) => {
                let Some((_, gaps)) = stored_gaps(&self.grammar, node) else { return false };
                let field = match &self.place {
                    Place::Field(field) => Some(field.as_str()),
                    _ => None,
                };
                gaps.before.iter().flatten().chain(&gaps.end).any(|token| field.is_none_or(|field| token.1 == field) && matches(token))
            }
            Place::Before | Place::After | Place::Earlier | Place::Later => {
                let Some(parent) = node.parent() else { return false };
                let Some((children, gaps)) = stored_gaps(&self.grammar, &parent) else { return false };
                let Some(at) = children.iter().position(|child| child.node_id() == node.node_id()) else { return false };
                match self.place {
                    Place::Before => gaps.before[at].last().is_some_and(matches),
                    Place::Earlier => gaps.before[..=at].iter().flatten().any(matches),
                    Place::Later => gaps.before[at + 1..].iter().flatten().chain(&gaps.end).any(matches),
                    // The gap after the node: before the next child, or the end. A stored token
                    // or an extra next is a stored gap, which the relation's own walk reads.
                    _ => match children.get(at + 1) {
                        Some(next) if next.is_named() => gaps.before[at + 1].first().is_some_and(matches),
                        Some(_) => false,
                        None => gaps.end.first().is_some_and(matches),
                    },
                }
            }
        }
    }
}

/// `parent`'s stored children and its gaps; `None` where its kind has no automaton.
pub(in crate::code_rule_dsl) fn stored_gaps<'t, 'g, D: Doc>(grammar: &'g TokenGrammar, parent: &Node<'t, D>) -> Option<(Vec<Node<'t, D>>, Gaps<'g>)> {
    let kind = parent.kind();
    let mut fields: HashMap<usize, &str> = HashMap::new();
    for field in grammar.fields_of(&kind) {
        for child in parent.field_children(field) {
            fields.insert(child.node_id(), field);
        }
    }
    let children: Vec<Node<'t, D>> = parent.children().collect();
    let kinds: Vec<(bool, String, &str)> = children.iter().map(|child| (child.is_named(), child.kind().into_owned(), fields.get(&child.node_id()).copied().unwrap_or(""))).collect();
    let gaps = grammar.gaps(&kind, kinds.iter().map(|(named, kind, field)| (*named, kind.as_str(), *field)))?;
    Some((children, gaps))
}
