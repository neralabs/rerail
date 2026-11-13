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
//! A target that says more of the token than its text is not a gap's question: the rule is
//! rewritten to ask the gap test and the structural test apart. Two such targets are still
//! answered here: the token's own relations to the tokens beside it (`follows: {regex: "^,$",
//! not: {follows: {regex: "^,$", stopBy: end}}}`: the first comma), read over the parent's
//! children and words in order ([`TokenCondition`]); and a token on the way to the next stored
//! child (`precedes: {regex: "^=$", precedes: X}`), which the compiler splits into this test, with
//! nothing between the token and the child, and the relation's own walk to X
//! (`compile::ast_rule::gap_split`). A `count` of a bare token regex counts the implied tokens too.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::tape::tokens::TokenGrammar;
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
    /// What the rule says of the token itself, beyond its text: its own relations to the tokens
    /// and stored children beside it under the same parent (`not: {follows: {regex: "^,$"}}`).
    pub(in crate::code_rule_dsl) token: Vec<TokenCondition>,
}

/// A condition on an implied token, read over its parent's children in order: the stored
/// children, with each gap's implied word before the child it precedes and the last word at the
/// end. A stored child is read by its text.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) enum TokenCondition {
    /// `follows` / `precedes` (a neighbour, or to the end) to a token regex, or under `not`.
    Token { negated: bool, place: Place, regex: Regex },
    /// The neighbour before (`Place::Before`) or after (`Place::After`) is a stored child: the
    /// gap's word is that token alone, and the relation's own walk reads the child.
    Stored(Place),
}

/// One place in a parent's children: an implied token (its text and field) or a stored child.
#[derive(Clone, Copy)]
enum Item<'g> {
    Token(&'g str, &'g str),
    Child(usize),
}

fn sequence<'g>(children: usize, gaps: &crate::code_rule_dsl::tape::tokens::Gaps<'g>) -> Vec<Item<'g>> {
    let mut items = Vec::new();
    for (at, word) in gaps.before.iter().enumerate().take(children) {
        items.extend(word.iter().map(|token| Item::Token(token.0, token.1)));
        items.push(Item::Child(at));
    }
    items.extend(gaps.end.iter().map(|token| Item::Token(token.0, token.1)));
    items
}

impl std::fmt::Debug for GapTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut test = f.debug_struct("GapTest");
        test.field("place", &self.place).field("regex", &self.regex.as_str());
        if !self.token.is_empty() {
            test.field("token", &self.token);
        }
        test.finish()
    }
}

impl GapTest {
    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        self.found(node, true) > 0
    }

    /// How many implied tokens the regex matches are at the test's place (`count`).
    pub(in crate::code_rule_dsl) fn count<D: Doc>(&self, node: &Node<'_, D>) -> usize {
        self.found(node, false)
    }

    fn found<D: Doc>(&self, node: &Node<'_, D>, first: bool) -> usize {
        // What a neighbour walk passes: the stored children its stop rule matches, and the implied
        // tokens its regexes do (no kind is an implied token's).
        let (past, skips) = match &self.past {
            None => (None, Vec::new()),
            Some(stop) => match self.passes(stop, 0) {
                Some(skips) => (Some(stop), skips),
                // A walk past more than kinds and tokens is not a gap's question.
                None => return 0,
            },
        };
        let skipped = |token: &str| skips.iter().any(|skip| skip.is_match(token));
        let owner = match &self.place {
            Place::Own | Place::Field(_) => node.clone(),
            _ => match node.parent() {
                Some(parent) => parent,
                None => return 0,
            },
        };
        let Some((children, gaps)) = stored_gaps(&self.grammar, &owner) else { return 0 };
        let items = sequence(children.len(), &gaps);
        // The token at `at` matches the regex and every condition on it.
        let token_at = |at: usize| match items[at] {
            Item::Token(text, _) => self.regex.is_match(text) && self.token.iter().all(|condition| condition.holds(&items, &children, at)),
            Item::Child(_) => false,
        };
        let count = |range: std::ops::Range<usize>| {
            let mut found = range.filter(|at| token_at(*at));
            if first { usize::from(found.next().is_some()) } else { found.count() }
        };
        if let Place::Own | Place::Field(_) = &self.place {
            let field = match &self.place {
                Place::Field(field) => Some(field.as_str()),
                _ => None,
            };
            let mut found = (0..items.len()).filter(|at| matches!(items[*at], Item::Token(_, of) if field.is_none_or(|field| of == field)) && token_at(*at));
            return if first { usize::from(found.next().is_some()) } else { found.count() };
        }
        let Some(at) = items.iter().position(|item| matches!(item, Item::Child(child) if children[*child].node_id() == node.node_id())) else { return 0 };
        // A stored child the walk passes: one its stop rule matches.
        let passed = |child: usize| {
            use ast_grep_core::matcher::Matcher;
            past.is_some_and(|stop| stop.match_node_with_env(children[child].clone(), &mut std::borrow::Cow::Owned(ast_grep_core::meta_var::MetaVarEnv::new())).is_some())
        };
        // The neighbour token: the first item the walk does not pass, if it is a token. An extra
        // right before or after the node is its neighbour, unless the walk passes it.
        let neighbour = |mut places: Box<dyn Iterator<Item = usize> + '_>| {
            places.find(|place| match items[*place] {
                Item::Token(text, _) => !skipped(text),
                Item::Child(child) => !passed(child),
            })
        };
        match self.place {
            Place::Before => neighbour(Box::new((0..at).rev())).map_or(0, |place| usize::from(token_at(place))),
            Place::After => neighbour(Box::new(at + 1..items.len())).map_or(0, |place| usize::from(token_at(place))),
            Place::Earlier => count(0..at),
            Place::Later => count(at + 1..items.len()),
            Place::Own | Place::Field(_) => unreachable!("a node's own tokens are read above"),
        }
    }
}

impl TokenCondition {
    /// Whether the condition holds of the token at `at` among `items`.
    fn holds<D: Doc>(&self, items: &[Item<'_>], children: &[Node<'_, D>], at: usize) -> bool {
        let neighbour = |place: &Place| match place {
            Place::Before => at.checked_sub(1).map(|before| items[before]),
            Place::After => items.get(at + 1).copied(),
            _ => None,
        };
        match self {
            Self::Stored(place) => matches!(neighbour(place), Some(Item::Child(_))),
            Self::Token { negated, place, regex } => {
                let matches = |item: &Item<'_>| match item {
                    Item::Token(text, _) => regex.is_match(text),
                    Item::Child(child) => regex.is_match(&children[*child].text()),
                };
                let found = match place {
                    Place::Earlier => items[..at].iter().any(matches),
                    Place::Later => items[at + 1..].iter().any(matches),
                    _ => neighbour(place).is_some_and(|item| matches(&item)),
                };
                found != *negated
            }
        }
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
