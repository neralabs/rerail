//! The gap test (t31): a relation to a bare token regex, where the token is one the tape leaves
//! out because the grammar implies it.
//!
//! A tape stores no token its grammar implies (RER-131): `f(a, b)` holds `f`, `a` and `b`, not the
//! parentheses or the comma. A rule that asks "is the node before me a `,`" can no longer find the
//! `,` as a node. It asks the grammar instead: the parent's stored children, walked through its
//! automaton, say which word each gap between them holds ([`TokenGrammar::gaps`]). That reads the
//! kinds and fields of one parent's children and static tables, never the source; nothing is
//! indexed. A parent's reading is kept while its file is evaluated (`memo::parent_gaps`): each of
//! its children, under every clause, asks about the same walk, which made again per child would
//! be quadratic in the parent's children.
//!
//! ```text
//! argument_list  f(a, b)       stored: a, b       gaps: before a "(", before b ",", end ")"
//! follows: { regex: "^,$" }    on b: the word before b ends in ","  -> holds
//! ```
//!
//! The compiler adds the test beside the relation's own walk (`compile::ast_rule`), so a stored
//! token still matches as a node and an implied one through its gap.
//!
//! A target that says more of the token than its text is not a gap's question: the rule is
//! rewritten to ask the gap test and the structural test apart.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::tape::tokens::{Gaps, TokenGrammar};
pub use crate::code_rule_dsl::tape::tokens::stored_gaps;
use ast_grep_core::{Doc, Node};
use regex::Regex;
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
pub struct GapTest {
    pub(in crate::code_rule_dsl) place: Place,
    pub(in crate::code_rule_dsl) regex: Regex,
    pub(in crate::code_rule_dsl) grammar: Arc<TokenGrammar>,
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    /// A neighbour walk's `stopBy: {not: X}`: X, which must name only kinds and tokens (by a bare
    /// regex), so the walk passes them. `None`: a plain neighbour, where an extra
    /// beside the node is the neighbour.
    pub(in crate::code_rule_dsl) past: Option<Box<AstMatcher>>,
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
        let read = |parent: &Node<'_, D>| super::memo::parent_gaps(Arc::as_ptr(&self.grammar) as usize, parent, || ParentGaps::read(&self.grammar, parent));
        // What a neighbour walk passes: the stored children its stop rule matches, and the implied
        // tokens its regexes do (no kind is an implied token's).
        let (past, skips) = match &self.past {
            None => (None, Vec::new()),
            Some(stop) => match self.passes(stop, 0) {
                Some(skips) => (Some(stop), skips),
                // A walk past more than kinds and tokens is not a gap's question.
                None => return false,
            },
        };
        let skipped = |token: &str| skips.iter().any(|skip| skip.is_match(token));
        match &self.place {
            Place::Own | Place::Field(_) => {
                let Some(read) = read(node) else { return false };
                let field = match &self.place {
                    Place::Field(field) => Some(field.as_str()),
                    _ => None,
                };
                read.gaps.before.iter().flatten().chain(&read.gaps.end).any(|token| field.is_none_or(|field| token.1 == field) && matches(token))
            }
            Place::Before | Place::After | Place::Earlier | Place::Later => {
                let Some(parent) = node.parent() else { return false };
                let Some(read) = read(&parent) else { return false };
                let Some(&at) = read.at.get(&node.node_id()) else { return false };
                let (gaps, stored) = (&read.gaps, read.extra.len());
                // A stored child the walk passes: one its stop rule matches. The gap's word comes
                // before the extras between the two stored children.
                let passed = |k: usize| {
                    use ast_grep_core::matcher::Matcher;
                    past.is_some_and(|stop| {
                        parent.children().nth(k).is_some_and(|child| stop.match_node_with_env(child, &mut std::borrow::Cow::Owned(ast_grep_core::meta_var::MetaVarEnv::new())).is_some())
                    })
                };
                match self.place {
                    // The first token back that the walk does not pass, through the gaps of the
                    // children it passes; a stored child is the relation's own walk's.
                    // An extra right before the node is its neighbour, unless the walk passes it.
                    Place::Before if past.is_none() && at > 0 && read.extra[at - 1] => false,
                    Place::Before => {
                        let mut k = at;
                        loop {
                            if let Some(token) = gaps.before[k].iter().rev().find(|token| !skipped(token.0)) {
                                return matches(token);
                            }
                            if k == 0 || !passed(k - 1) {
                                return false;
                            }
                            k -= 1;
                        }
                    }
                    Place::Earlier => gaps.before[..=at].iter().flatten().any(matches),
                    Place::Later => gaps.before[at + 1..].iter().flatten().chain(&gaps.end).any(matches),
                    // The gap after the node: the first token on that the walk does not pass,
                    // before the next children, or at the end.
                    _ => {
                        let mut k = at + 1;
                        loop {
                            let word = if k < stored { &gaps.before[k] } else { &gaps.end };
                            if let Some(token) = word.iter().find(|token| !skipped(token.0)) {
                                return matches(token);
                            }
                            if k >= stored || !passed(k) {
                                return false;
                            }
                            k += 1;
                        }
                    }
                }
            }
        }
    }
}

/// A parent's stored children and the gaps between them (`stored_gaps`), as the gap tests read
/// them: read once per parent of the file being evaluated (`memo::parent_gaps`).
pub(super) struct ParentGaps {
    /// Each stored child's position among them, by node id.
    at: rustc_hash::FxHashMap<usize, usize>,
    /// Whether each stored child is one of the grammar's extras.
    extra: Vec<bool>,
    /// The gaps' words, in the grammar's interned texts.
    gaps: Gaps<'static>,
}

impl ParentGaps {
    /// `parent`'s reading; `None` where its kind has no automaton or the automaton refuses its
    /// children.
    fn read<D: Doc>(grammar: &TokenGrammar, parent: &Node<'_, D>) -> Option<Self> {
        let (children, gaps) = stored_gaps(grammar, parent)?;
        let word = |word: Vec<(&str, &str)>| word.into_iter().map(|(token, field)| (grammar.intern(token), grammar.intern(field))).collect::<Vec<_>>();
        Some(Self {
            at: children.iter().enumerate().map(|(at, child)| (child.node_id(), at)).collect(),
            extra: children.iter().map(|child| grammar.is_extra(&child.kind())).collect(),
            gaps: Gaps { before: gaps.before.into_iter().map(word).collect(), end: word(gaps.end) },
        })
    }
}

impl GapTest {
    /// The token regexes `rule` passes, where it is made of kinds (which no implied token has)
    /// and tokens (by a bare regex); `None` where it says more.
    fn passes(&self, rule: &AstMatcher, depth: usize) -> Option<Vec<Regex>> {
        if depth > 32 {
            return None;
        }
        match rule {
            AstMatcher::Kind(_) | AstMatcher::Supertype(_) => Some(Vec::new()),
            AstMatcher::Regex(regex) => Some(vec![regex.clone()]),
            AstMatcher::Any(rules) => rules.iter().try_fold(Vec::new(), |mut all, rule| {
                all.extend(self.passes(rule, depth + 1)?);
                Some(all)
            }),
            AstMatcher::Util(util) => util.with_target(|rule| self.passes(rule, depth + 1)).flatten(),
            _ => None,
        }
    }
}
