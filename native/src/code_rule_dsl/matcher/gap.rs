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
use crate::code_rule_dsl::regexes::Regex;
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
    /// Beside a neighbour relation's own walk (`compile::ast_rule::with_gap`): the test holds
    /// where the neighbour at its place is a stored child, not a token the tape leaves out,
    /// whatever the regex. On a full tree the implied token would be the neighbour, so the walk's
    /// next stored node is not (`for (;; i++);`: after `i++` comes `)`, not the body's `;`).
    pub(in crate::code_rule_dsl) stored: bool,
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
        if self.stored {
            test.field("stored", &true);
        }
        test.finish()
    }
}

/// What a gap test found under the token's parent (`owner`): the items in order (its reading),
/// the implied tokens that pass (`at`, item indices) and, for a neighbour walk, the item it
/// stopped at.
struct Found<'t, D: Doc> {
    owner: Node<'t, D>,
    read: Arc<ParentGaps>,
    /// The owner's stored children, collected where a test reads one.
    children: std::cell::OnceCell<Vec<Node<'t, D>>>,
    at: Vec<usize>,
    neighbour: Option<usize>,
}

impl<'t, D: Doc> Found<'t, D> {
    fn children(&self) -> &[Node<'t, D>] {
        self.children.get_or_init(|| self.owner.children().collect())
    }
}

/// A parent's stored children and the implied tokens between them in order, as the gap tests read
/// them: read once per parent of the file being evaluated (`memo::parent_gaps`).
pub(super) struct ParentGaps {
    /// The children and the gaps' tokens (`interleaved`), in the grammar's interned texts.
    items: Vec<Item<'static>>,
    /// Each stored child's index among `items`, by node id.
    child_at: rustc_hash::FxHashMap<usize, usize>,
}

impl GapTest {
    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let found = self.found(node, true);
        if self.stored {
            // Where the gaps cannot be read the node keeps its tokens, and the walk reads them.
            return found.is_none_or(|found| found.neighbour.is_none_or(|at| matches!(found.read.items[at], Item::Child(_))));
        }
        found.is_some_and(|found| !found.at.is_empty())
    }

    /// How many implied tokens the regex matches are at the test's place (`count`).
    pub(in crate::code_rule_dsl) fn count<D: Doc>(&self, node: &Node<'_, D>) -> usize {
        self.found(node, false).map_or(0, |found| found.at.len())
    }

    /// The token at the test's place that the regex matches, its byte range in the file and its
    /// text: the first implied one, or a stored token where the tape keeps it (the relation's own
    /// walk reads that one). An implied token's range is read from the source between its
    /// parent's stored children: the gap holds only its word, whitespace and extras. `None` where
    /// no token matches.
    pub(in crate::code_rule_dsl) fn token<D: Doc>(&self, node: &Node<'_, D>) -> Option<(std::ops::Range<usize>, String)> {
        let found = self.found(node, true)?;
        let children = found.children();
        let items = &found.read.items;
        let stored = |at: &usize| matches!(items[*at], Item::Child(child) if !children[child].is_named() && self.regex.is_match(&children[child].text()));
        let candidate = match &self.place {
            Place::Own | Place::Field(_) => (0..items.len()).find(|at| found.at.first() == Some(at) || stored(at)),
            Place::Before | Place::After => found.at.first().copied().or(found.neighbour.filter(stored)),
            Place::Earlier | Place::Later => found.at.first().copied(),
        }?;
        match items[candidate] {
            Item::Child(child) => Some((children[child].range(), children[child].text().into_owned())),
            Item::Token(text, _) => {
                let start = self.token_starts(&found.owner, children, items)[candidate]?;
                Some((start..start + text.len(), text.to_owned()))
            }
        }
    }

    /// The start of each implied token among `items`, by its place in the source: a gap's tokens
    /// lie after the named child before it that is not an extra (or the parent's start), in
    /// order, with only white space, extras and stored tokens between them. `None` for a stored
    /// child, and for a token the source does not hold where the grammar says.
    fn token_starts<D: Doc>(&self, owner: &Node<'_, D>, children: &[Node<'_, D>], items: &[Item<'_>]) -> Vec<Option<usize>> {
        let passed = |child: &Node<'_, D>| !child.is_named() || self.grammar.is_extra(&child.kind());
        let base = owner.range().start;
        let text = owner.text();
        let mut starts = vec![None; items.len()];
        let mut pos = Some(base);
        for (index, item) in items.iter().enumerate() {
            match item {
                Item::Child(child) if !passed(&children[*child]) => pos = Some(children[*child].range().end),
                Item::Child(_) => {}
                Item::Token(token, _) => {
                    pos = pos.and_then(|mut pos| {
                        loop {
                            let rest = text.get(pos.checked_sub(base)?..)?;
                            pos += rest.len() - rest.trim_start().len();
                            match children.iter().find(|child| child.range().start == pos && passed(child)) {
                                Some(child) => pos = child.range().end,
                                None => break,
                            }
                        }
                        text.get(pos - base..)?.starts_with(token).then_some(pos)
                    });
                    starts[index] = pos;
                    pos = pos.map(|pos| pos + token.len());
                }
            }
        }
        starts
    }

    /// `items` with each gap's implied tokens among the extras and stored tokens beside them in
    /// source order. `sequence` puts a gap's word last before the named child it precedes, but the
    /// source may hold its tokens before or between them (`{/* c */}`: `{`, the comment, `}`;
    /// `for (k in o)`: the stored `for`, then `(`).
    fn interleaved<'g, D: Doc>(&self, owner: &Node<'_, D>, children: &[Node<'_, D>], items: Vec<Item<'g>>) -> Vec<Item<'g>> {
        let passed = |child: usize| !children[child].is_named() || self.grammar.is_extra(&children[child].kind());
        // Only a run holding both a passed child and an implied token can be out of order.
        let mut mixed = false;
        let (mut child_in_run, mut token_in_run) = (false, false);
        for item in &items {
            match *item {
                Item::Child(child) if passed(child) => child_in_run = true,
                Item::Child(_) => (child_in_run, token_in_run) = (false, false),
                Item::Token(..) => token_in_run = true,
            }
            mixed |= child_in_run && token_in_run;
        }
        if !mixed {
            return items;
        }
        let starts = self.token_starts(owner, children, &items);
        let key = |at: usize| match items[at] {
            Item::Child(child) => Some(children[child].range().start),
            Item::Token(..) => starts[at],
        };
        let mut order: Vec<usize> = Vec::with_capacity(items.len());
        let mut run: Vec<usize> = Vec::new();
        for at in 0..=items.len() {
            let ends = at == items.len() || matches!(items[at], Item::Child(child) if !passed(child));
            if !ends {
                run.push(at);
                continue;
            }
            // A run of extras, stored and implied tokens: by source, where every token was found.
            if run.iter().all(|&at| key(at).is_some()) {
                run.sort_by_key(|&at| key(at));
            }
            order.append(&mut run);
            if at < items.len() {
                order.push(at);
            }
        }
        order.into_iter().map(|at| items[at]).collect()
    }

    /// `owner`'s reading: its children and the gaps' tokens in order; `None` where its kind has no
    /// automaton or the automaton refuses its children.
    fn read<D: Doc>(&self, owner: &Node<'_, D>) -> Option<ParentGaps> {
        let (children, gaps) = stored_gaps(&self.grammar, owner)?;
        let items: Vec<Item<'static>> = self
            .interleaved(owner, &children, sequence(children.len(), &gaps))
            .into_iter()
            .map(|item| match item {
                Item::Token(text, field) => Item::Token(self.grammar.intern(text), self.grammar.intern(field)),
                Item::Child(child) => Item::Child(child),
            })
            .collect();
        let child_at = items
            .iter()
            .enumerate()
            .filter_map(|(at, item)| match item {
                Item::Child(child) => Some((children[*child].node_id(), at)),
                Item::Token(..) => None,
            })
            .collect();
        Some(ParentGaps { items, child_at })
    }

    fn found<'t, D: Doc>(&self, node: &Node<'t, D>, first: bool) -> Option<Found<'t, D>> {
        // What a neighbour walk passes: the stored children its stop rule matches, and the implied
        // tokens its regexes do (no kind is an implied token's).
        let (past, skips) = match &self.past {
            None => (None, Vec::new()),
            // A walk past more than kinds and tokens is not a gap's question.
            Some(stop) => (Some(stop), self.passes(stop, 0)?),
        };
        let skipped = |token: &str| skips.iter().any(|skip| skip.is_match(token));
        let owner = match &self.place {
            Place::Own | Place::Field(_) => node.clone(),
            _ => node.parent()?,
        };
        let read = super::memo::parent_gaps(Arc::as_ptr(&self.grammar) as usize, &owner, || self.read(&owner))?;
        let items = &read.items;
        let children = std::cell::OnceCell::new();
        let child = |k: usize| children.get_or_init(|| owner.children().collect::<Vec<_>>())[k].clone();
        // The token at `at` matches the regex and every condition on it.
        let token_at = |at: usize| match items[at] {
            Item::Token(text, _) => self.regex.is_match(text) && self.token.iter().all(|condition| condition.holds(items, &child, at)),
            Item::Child(_) => false,
        };
        let collect = |range: &mut dyn Iterator<Item = usize>| -> Vec<usize> {
            let found = range.filter(|at| token_at(*at));
            if first { found.take(1).collect() } else { found.collect() }
        };
        let (at, neighbour) = if let Place::Own | Place::Field(_) = &self.place {
            let field = match &self.place {
                Place::Field(field) => Some(field.as_str()),
                _ => None,
            };
            (collect(&mut (0..items.len()).filter(|at| matches!(items[*at], Item::Token(_, of) if field.is_none_or(|field| of == field)))), None)
        } else {
            let at = *read.child_at.get(&node.node_id())?;
            // A stored child the walk passes: one its stop rule matches.
            let passed = |k: usize| {
                use ast_grep_core::matcher::Matcher;
                past.is_some_and(|stop| stop.match_node_with_env(child(k), &mut std::borrow::Cow::Owned(ast_grep_core::meta_var::MetaVarEnv::new())).is_some())
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
                Place::Before | Place::After => {
                    let place = if self.place == Place::Before { neighbour(Box::new((0..at).rev())) } else { neighbour(Box::new(at + 1..items.len())) };
                    (place.filter(|place| token_at(*place)).into_iter().collect(), place)
                }
                Place::Earlier => (collect(&mut (0..at)), None),
                Place::Later => (collect(&mut (at + 1..items.len())), None),
                Place::Own | Place::Field(_) => unreachable!("a node's own tokens are read above"),
            }
        };
        Some(Found { owner, read, children, at, neighbour })
    }
}

impl TokenCondition {
    /// Whether the condition holds of the token at `at` among `items`.
    fn holds<'t, D: Doc>(&self, items: &[Item<'_>], child: &dyn Fn(usize) -> Node<'t, D>, at: usize) -> bool {
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
                    Item::Child(k) => regex.is_match(&child(*k).text()),
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
