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
//!
//! A target that is a token and more (`follows: {regex: "^,$", follows: {matches: first-arg}}`: the
//! comma after the first argument) is a [`Walk`]: the relation walks the parent's parts, its
//! stored children with the implied tokens between them (`tree::Tree::parts`), testing a stored
//! part with the target as compiled and an implied token with [`token_matches`]: what the target
//! says of a token's text and of the parts around it. Nothing else holds of a token.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy};
use crate::code_rule_dsl::tree::{Part, Tree};
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use std::borrow::Cow;
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
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    /// A neighbour walk's `stopBy: {not: X}`: X, which must name only extras (comments), so the
    /// walk passes them. `None`: a plain neighbour, where an extra beside the node is the neighbour.
    pub(in crate::code_rule_dsl) past: Option<Box<AstMatcher>>,
    /// A target that is a token and more: the relation, walked over the parts; `place`, `regex`
    /// and `past` are then unused.
    pub(in crate::code_rule_dsl) walk: Option<Walk>,
}

/// A relation over a parent's parts ([`GapTest::walk`]).
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Walk {
    pub(in crate::code_rule_dsl) relation: Relation,
    pub(in crate::code_rule_dsl) stop: StopBy,
    /// The relation's target, as compiled.
    pub(in crate::code_rule_dsl) target: AstMatcher,
    /// Whether the target can match a token the grammar implies ([`names_token`]), read from the
    /// compiled target on first use, once its utilities are filled.
    pub(in crate::code_rule_dsl) names_token: std::sync::OnceLock<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Relation {
    /// `follows`: the parts before the node.
    Follows,
    /// `precedes`: the parts after it.
    Precedes,
    /// `has` (a neighbour): the node's own parts.
    Has,
    /// `has` past the neighbours (`stopBy: end` or a stop rule): the parts of the node and of
    /// each node below it the walk reaches, as ast-grep descends.
    Below,
}

impl std::fmt::Debug for GapTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GapTest").field("place", &self.place).field("regex", &self.regex.as_str()).finish()
    }
}

impl GapTest {
    /// Whether the relation holds of `node`: a walk over the parts, or the gap's word at its place.
    pub(in crate::code_rule_dsl) fn matches<'t, D: Doc>(&self, node: &Node<'t, D>, env: &mut Cow<MetaVarEnv<'t, D>>) -> bool {
        let Some(walk) = &self.walk else { return self.holds(node) };
        if !self.can_hold() {
            return false;
        }
        match walk.relation {
            Relation::Has => {
                let parts: Vec<Part<'t, D>> = node.parts().collect();
                (0..parts.len()).any(|at| parts[at].node().is_none() && token_matches(&walk.target, &parts, at, node, env, 0))
            }
            Relation::Below => below(node, &walk.stop, &walk.target, env, 0),
            Relation::Follows | Relation::Precedes => {
                let Some(parent) = node.parent() else { return false };
                let parts: Vec<Part<'t, D>> = parent.parts().collect();
                let Some(at) = parts.iter().position(|part| part.named().is_some_and(|named| named.node_id() == node.node_id())) else { return false };
                sibling_walk(&parts, at, walk.relation == Relation::Precedes, &walk.stop, &walk.target, &parent, env, true)
            }
        }
    }

    /// Whether the test can hold anywhere: a walk only where its target can pass an implied
    /// token ([`names_token`], read once its utilities are filled).
    pub(in crate::code_rule_dsl) fn can_hold(&self) -> bool {
        self.walk.as_ref().is_none_or(|walk| *walk.names_token.get_or_init(|| names_token(&walk.target, &self.grammar, 0)))
    }

    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let matches = |token: &(&str, &str)| self.regex.is_match(token.0);
        let past = match &self.past {
            None => false,
            Some(stop) if self.only_extras(stop, 0) => true,
            // A walk that stops at other kinds is not a gap's question.
            Some(_) => return false,
        };
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
                    // An extra right before the node is its neighbour, unless the walk passes it;
                    // the gap's word comes before the extras between the two stored children.
                    Place::Before if !past && at > 0 && self.grammar.is_extra(&children[at - 1].kind()) => false,
                    Place::Before => gaps.before[at].last().is_some_and(matches),
                    Place::Earlier => gaps.before[..=at].iter().flatten().any(matches),
                    Place::Later => gaps.before[at + 1..].iter().flatten().chain(&gaps.end).any(matches),
                    // The gap after the node: before the next child, or the end. A stored token
                    // or an extra next is a stored gap, which the relation's own walk reads.
                    _ => {
                        let next = children[at + 1..].iter().position(|child| !(past && self.grammar.is_extra(&child.kind()))).map(|step| at + 1 + step);
                        match next.map(|next| (next, &children[next])) {
                            Some((next, child)) if child.is_named() && !self.grammar.is_extra(&child.kind()) => gaps.before[next].first().is_some_and(matches),
                            Some(_) => false,
                            None => gaps.end.first().is_some_and(matches),
                        }
                    }
                }
            }
        }
    }
}

impl GapTest {
    /// Whether `rule` matches nodes by kind alone, and only the grammar's extras.
    fn only_extras(&self, rule: &AstMatcher, depth: usize) -> bool {
        use ast_grep_core::matcher::Matcher;
        if depth > 32 {
            return false;
        }
        match rule {
            AstMatcher::Kind(kind) => kind.potential_kinds().is_some_and(|set| {
                set.iter().all(|id| u16::try_from(id).ok().and_then(|id| self.dialect.kind_name(id)).is_some_and(|name| self.grammar.is_extra(name)))
            }),
            AstMatcher::Any(rules) => rules.iter().all(|rule| self.only_extras(rule, depth + 1)),
            AstMatcher::Util(util) => util.with_target(|rule| self.only_extras(rule, depth + 1)).unwrap_or(false),
            _ => false,
        }
    }
}

/// Whether `rule` holds of part `at` of `parent`'s parts: a stored part as the rule is compiled
/// (its bindings kept where it matches), an implied token by [`token_matches`].
fn part_matches<'t, D: Doc>(rule: &AstMatcher, parts: &[Part<'t, D>], at: usize, parent: &Node<'t, D>, env: &mut Cow<MetaVarEnv<'t, D>>) -> bool {
    match parts[at].node() {
        Some(node) => {
            let mut trial = env.clone();
            let matched = rule.match_node_with_env(node, &mut trial).is_some();
            if matched {
                *env = trial;
            }
            matched
        }
        None => token_matches(rule, parts, at, parent, env, 0),
    }
}

/// Whether `target` holds of an implied token of `node` or, depth first, of a stored node below
/// it: a named child is descended into unless it matched `stop`, as ast-grep descends. Stored
/// parts are the relation's own walk's (beside this one).
fn below<'t, D: Doc>(node: &Node<'t, D>, stop: &StopBy, target: &AstMatcher, env: &mut Cow<MetaVarEnv<'t, D>>, depth: usize) -> bool {
    if depth > 256 {
        return false;
    }
    let parts: Vec<Part<'t, D>> = node.parts().collect();
    for at in 0..parts.len() {
        if parts[at].node().is_none() && token_matches(target, &parts, at, node, env, 0) {
            return true;
        }
        let Some(child) = parts[at].named() else { continue };
        let stopped = match stop {
            StopBy::Neighbor => true,
            StopBy::End => false,
            StopBy::Rule(rule) => rule.match_node_with_env(child.clone(), &mut Cow::Owned(MetaVarEnv::new())).is_some(),
        };
        if !stopped && below(&child, stop, target, env, depth + 1) {
            return true;
        }
    }
    false
}

/// `follows` (backward) or `precedes` (`forward`) from part `at`, as ast-grep's sibling walk:
/// each part in turn is tested, and the walk ends after the neighbour, at the end, or after the
/// first part the stop rule matches (that part is still tested). `tokens_only`: the relation's
/// own walk answers for its stored parts (the walk is beside it, `compile::ast_rule::with_gap`),
/// so only implied tokens are tested against the target.
#[allow(clippy::too_many_arguments)]
fn sibling_walk<'t, D: Doc>(parts: &[Part<'t, D>], at: usize, forward: bool, stop: &StopBy, target: &AstMatcher, parent: &Node<'t, D>, env: &mut Cow<MetaVarEnv<'t, D>>, tokens_only: bool) -> bool {
    let order: Box<dyn Iterator<Item = usize>> = if forward { Box::new(at + 1..parts.len()) } else { Box::new((0..at).rev()) };
    for next in order {
        let tested = if tokens_only && parts[next].node().is_some() { false } else { part_matches(target, parts, next, parent, env) };
        if tested {
            return true;
        }
        let stopped = match stop {
            StopBy::Neighbor => true,
            StopBy::End => false,
            StopBy::Rule(rule) => part_matches(rule, parts, next, parent, &mut Cow::Owned(MetaVarEnv::new())),
        };
        if stopped {
            return false;
        }
    }
    false
}

/// Whether `rule` holds of the implied token at part `at` of `parent`'s parts: its regexes read the
/// token's text, `not`, `all` and `any` combine, `follows` and `precedes` walk the parts around it,
/// `inside` reads `parent` (and, past a neighbour, its ancestors), a utility its rule. Nothing else
/// (a kind, a pattern, children, a semantic test) holds of a token.
fn token_matches<'t, D: Doc>(rule: &AstMatcher, parts: &[Part<'t, D>], at: usize, parent: &Node<'t, D>, env: &mut Cow<MetaVarEnv<'t, D>>, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let text = parts[at].kind();
    match rule {
        AstMatcher::Regex(regex) => regex.is_match(&text),
        AstMatcher::Not(rule) => !token_matches(rule, parts, at, parent, &mut Cow::Owned(env.as_ref().clone()), depth + 1),
        AstMatcher::All(rules) => rules.iter().all(|rule| token_matches(rule, parts, at, parent, env, depth + 1)),
        AstMatcher::Any(rules) => rules.iter().any(|rule| token_matches(rule, parts, at, parent, env, depth + 1)),
        AstMatcher::Util(util) => util.with_target(|rule| token_matches(rule, parts, at, parent, env, depth + 1)).unwrap_or(false),
        AstMatcher::Follows(related, stop) => sibling_walk(parts, at, false, stop, related.rule(), parent, env, false),
        AstMatcher::Precedes(related, stop) => sibling_walk(parts, at, true, stop, related.rule(), parent, env, false),
        AstMatcher::Inside(related, stop) => {
            let mut trial = env.clone();
            let found = match stop {
                StopBy::Neighbor => related.rule().match_node_with_env(parent.clone(), &mut trial).is_some(),
                StopBy::End => std::iter::once(parent.clone()).chain(parent.ancestors()).any(|up| related.rule().match_node_with_env(up, &mut trial).is_some()),
                StopBy::Rule(stop) => {
                    let mut found = false;
                    for up in std::iter::once(parent.clone()).chain(parent.ancestors()) {
                        if related.rule().match_node_with_env(up.clone(), &mut trial).is_some() {
                            found = true;
                            break;
                        }
                        if stop.match_node_with_env(up, &mut Cow::Owned(MetaVarEnv::new())).is_some() {
                            break;
                        }
                    }
                    found
                }
            };
            if found {
                *env = trial;
            }
            found
        }
        AstMatcher::Semantic(super::Semantic::Gap(test)) if test.walk.is_none() => false,
        _ => false,
    }
}

/// Whether `rule` tests a token the grammar implies: a regex that matches one of its texts, with
/// nothing beside it that no token passes (a kind, a pattern, children), through `all`, `any`
/// and utilities.
fn names_token(rule: &AstMatcher, grammar: &TokenGrammar, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match rule {
        AstMatcher::Regex(regex) => grammar.tests_implied(regex),
        AstMatcher::All(rules) => rules.iter().any(|rule| names_token(rule, grammar, depth + 1)) && rules.iter().all(|rule| token_possible(rule, grammar, depth + 1)),
        AstMatcher::Any(rules) => rules.iter().any(|rule| names_token(rule, grammar, depth + 1)),
        AstMatcher::Util(util) => util.with_target(|rule| names_token(rule, grammar, depth + 1)).unwrap_or(false),
        _ => false,
    }
}

/// Whether `rule` may hold of some implied token: what [`token_matches`] can say yes to.
fn token_possible(rule: &AstMatcher, grammar: &TokenGrammar, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match rule {
        AstMatcher::Regex(regex) => grammar.implied_texts().iter().any(|token| regex.is_match(token)),
        AstMatcher::Not(_) | AstMatcher::Follows(..) | AstMatcher::Precedes(..) | AstMatcher::Inside(..) => true,
        AstMatcher::All(rules) => rules.iter().all(|rule| token_possible(rule, grammar, depth + 1)),
        AstMatcher::Any(rules) => rules.iter().any(|rule| token_possible(rule, grammar, depth + 1)),
        AstMatcher::Util(util) => util.with_target(|rule| token_possible(rule, grammar, depth + 1)).unwrap_or(false),
        _ => false,
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
