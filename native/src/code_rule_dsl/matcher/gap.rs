//! The gap test (t31): a relation to a bare token regex, where the token is one the tape leaves
//! out because the grammar implies it.
//!
//! A tape stores no token its grammar implies (RER-131): `f(a, b)` holds `f`, `a` and `b`, not the
//! parentheses or the comma. A rule that asks "is the node before me a `,`" can no longer find the
//! `,` as a node. It asks the grammar instead: the parent's stored children, walked through its
//! automaton, say which word each gap between them holds ([`TokenGrammar::gaps`]). That reads the
//! kinds and fields of one parent's children and static tables, never the source. A tape's facts
//! make each parent's gaps once, the first time a test asks (`GapScope`, a scratch of their call:
//! Q134), not for each child; nothing outlives the call.
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
use std::cell::RefCell;
use std::rc::Rc;
use crate::code_rule_dsl::tape::tokens::Gaps;

/// A parent's stored children (their node ids, ascending, and which are extras) and its gaps, as `stored_gaps` gives
/// them, made once a tape's call (`GapScope`, Q134), and the gaps an `Earlier` or `Later` test found matching, by test.
pub(in crate::code_rule_dsl) struct ParentGaps {
    ids: Vec<usize>,
    extra: Vec<bool>,
    gaps: Gaps<'static>,
    /// Per test asked (its address while the tape's call holds it), the first and the last gap with a token it matches
    /// (`before`'s, then `end` as the last), `NONE` for none: an `Earlier` or `Later` test is then a comparison, not a
    /// pass over the gaps before or after each child.
    matching: RefCell<Vec<(usize, u32, u32)>>,
}

const NONE: u32 = u32::MAX;

/// The parents' gaps made (`parent_gaps`), on any thread, for tests: a scope makes each parent's once.
#[cfg(any(test, feature = "test-hooks"))]
pub static GAPS_MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn parent_gaps<D: Doc>(grammar: &'static TokenGrammar, parent: &Node<'_, D>) -> Option<ParentGaps> {
    #[cfg(any(test, feature = "test-hooks"))]
    GAPS_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (children, gaps) = stored_gaps(grammar, parent)?;
    Some(ParentGaps {
        ids: children.iter().map(Node::node_id).collect(),
        extra: children.iter().map(|child| grammar.is_extra(&child.kind())).collect(),
        gaps,
        matching: RefCell::new(Vec::new()),
    })
}

impl ParentGaps {
    /// The first and last gap holding a token `test` matches (`matching`), found once a test.
    fn matching(&self, test: &GapTest) -> (u32, u32) {
        let key = std::ptr::from_ref(test) as usize;
        if let Some(&(_, first, last)) = self.matching.borrow().iter().find(|(known, ..)| *known == key) {
            return (first, last);
        }
        let holds = |word: &[(&str, &str)]| word.iter().any(|token| test.regex.is_match(token.0));
        let words = self.gaps.before.iter().map(Vec::as_slice).chain(std::iter::once(self.gaps.end.as_slice()));
        let (mut first, mut last) = (NONE, NONE);
        for (at, word) in words.enumerate() {
            if holds(word) {
                first = first.min(at as u32);
                last = at as u32;
            }
        }
        self.matching.borrow_mut().push((key, first, last));
        (first, last)
    }
}

/// A tape's parents' gaps by parent node id, made when a gap test first asks (`None` until then; `Some(None)` where
/// the parent has none), per dialect whose automata made them (a test's own: a guest document's tape is its host's
/// grammar's).
struct TapeGaps {
    nodes: usize,
    by: Vec<(GrammarDialect, Vec<Option<Option<Rc<ParentGaps>>>>)>,
}

thread_local! {
    static TAPE_GAPS: RefCell<Option<TapeGaps>> = const { RefCell::new(None) };
}

/// One tape's parents' gaps for its facts' gap tests (a scratch of the call, on this thread until dropped:
/// `fact_rows`), so a parent's are made once, not for each child a test asks of (Q134: a parent of n children cost
/// n²). Outside a scope each test makes its parent's.
pub struct GapScope {
    previous: Option<TapeGaps>,
}

impl GapScope {
    /// The scope for a tape of `nodes` nodes (node ids below it).
    pub fn install(nodes: usize) -> Self {
        Self { previous: TAPE_GAPS.with(|gaps| gaps.replace(Some(TapeGaps { nodes, by: Vec::new() }))) }
    }
}

impl Drop for GapScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TAPE_GAPS.with(|gaps| *gaps.borrow_mut() = previous);
    }
}

/// The parents' gaps of the file being matched (`resolve::file_index`), outside a tape's `GapScope`: a sibling walk
/// reads its parent's (`relation::charged`) on every path that matches a file, and a parent of n children made once a
/// walk cost n² (Q134's shape: 6,125 makings of one 20,000-element list's in one clause). Keyed by the tape (its
/// source), its dialect and the parent: a file's embedded languages are tapes of their own.
type FileGaps = RefCell<std::collections::HashMap<(usize, GrammarDialect, usize), Option<Rc<ParentGaps>>>>;

/// `parent`'s gaps: the tape scope's or the file's, made the first time they are asked, else made here.
fn gaps_of<D: Doc>(dialect: GrammarDialect, grammar: &'static TokenGrammar, parent: &Node<'_, D>) -> Option<Rc<ParentGaps>> {
    let id = parent.node_id();
    TAPE_GAPS.with(|gaps| {
        let mut gaps = gaps.borrow_mut();
        let Some(tape) = gaps.as_mut().filter(|tape| id < tape.nodes) else {
            drop(gaps);
            let Some(doc) = crate::code_rule_dsl::tape::tape_doc(parent) else { return parent_gaps(grammar, parent).map(Rc::new) };
            let key = (std::sync::Arc::as_ptr(doc.document_source()) as usize, dialect, id);
            let file = super::resolve::file_index(FileGaps::default);
            if let Some(made) = file.borrow().get(&key) {
                return made.clone();
            }
            let made = parent_gaps(grammar, parent).map(Rc::new);
            file.borrow_mut().insert(key, made.clone());
            return made;
        };
        let at = match tape.by.iter().position(|(made, _)| *made == dialect) {
            Some(at) => at,
            None => {
                tape.by.push((dialect, vec![None; tape.nodes]));
                tape.by.len() - 1
            }
        };
        tape.by[at].1[id].get_or_insert_with(|| parent_gaps(grammar, parent).map(Rc::new)).clone()
    })
}

/// The implied tokens a sibling walk passes between stored children of one parent, counted as the tree-sitter tree
/// holds them: the candidates a walk over a full tree tests between two of the tape's (RER-131, PR #508's 7529228ce).
/// A named-only tape leaves them out, so its walk tests fewer candidates for the same siblings, and its work budget,
/// spent per candidate, bought more work than a full tree's: the walk charges these (`relation::charged`). Read from
/// the parent's gaps (`gaps_of`, made once a parent within a tape's call); none for a document that is not a tape,
/// and where the parent's kind has no automaton or its gaps cannot be read (its tokens are then stored, and walked).
pub(super) struct ImpliedPassed(Option<Rc<ParentGaps>>);

impl ImpliedPassed {
    /// The gaps of `node`'s parent.
    pub(super) fn of<D: Doc>(node: &Node<'_, D>) -> Self {
        let read = crate::code_rule_dsl::tape::tape_doc(node).and_then(|doc| {
            let dialect = *doc.get_lang();
            let grammar = TokenGrammar::of_dialect_static(dialect)?;
            gaps_of(dialect, grammar, &node.parent()?)
        });
        Self(read)
    }

    /// The implied tokens from the child `from` to the child `to` (node ids), walking `forward` or back; `to`
    /// `None`: on to the parent's end, or back to its start. A gap's tokens lie before the child they are kept with
    /// (`Gaps::before`), the last child's after it (`Gaps::end`).
    pub(super) fn between(&self, from: usize, to: Option<usize>, forward: bool) -> usize {
        let Some(read) = &self.0 else { return 0 };
        let Ok(at) = read.ids.binary_search(&from) else { return 0 };
        let before = |k: usize| read.gaps.before.get(k).map_or(0, Vec::len);
        let to = match to {
            Some(to) => match read.ids.binary_search(&to) {
                Ok(end) => Some(end),
                Err(_) => return 0,
            },
            None => None,
        };
        match (forward, to) {
            (true, Some(end)) if end > at => (at + 1..=end).map(before).sum(),
            (true, None) => (at + 1..read.ids.len()).map(before).sum::<usize>() + read.gaps.end.len(),
            (false, Some(end)) if end < at => (end + 1..=at).map(before).sum(),
            (false, None) => (0..=at).map(before).sum(),
            _ => 0,
        }
    }
}

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
    /// Its dialect, whose automata are decoded when a test first runs on a tape (`TokenGrammar::of_dialect_static`):
    /// compiling a rule asks the census, and a warm run that tests no tape decodes nothing.
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    /// A neighbour walk's `stopBy: {not: X}`: X, which must name only kinds and tokens (by a bare
    /// regex), so the walk passes them. `None`: a plain neighbour, where an extra
    /// beside the node is the neighbour.
    pub(in crate::code_rule_dsl) past: Option<Box<AstMatcher>>,
    /// What the target says of the token beside its text, each a relation from the token found to a bare token regex
    /// (`follows: {regex: "^,$", not: {follows: {stopBy: end, regex: "^,$"}}}`: the comma right before the node is its
    /// parent's first). Asked of the parent's tokens, its gaps' and its stored children's (by their text): a `Before`
    /// or `After` test's alone.
    pub(in crate::code_rule_dsl) token: Vec<TokenCondition>,
    /// Beside a neighbour relation's own walk (`compile::ast_rule::with_gap`): the test holds
    /// where the neighbour at its place is a stored child, not a token the tape leaves out,
    /// whatever the regex. On a full tree the implied token would be the neighbour, so the walk's
    /// next stored node is not (`for (;; i++);`: after `i++` comes `)`, not the body's `;`).
    pub(in crate::code_rule_dsl) stored: bool,
}

/// One condition on the token a gap test found (`GapTest::token`), read along the parent's tokens and stored children.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) enum TokenCondition {
    /// A token `regex` matches is at `place` from it (`Before`, `After`, `Earlier` or `Later`), or with `negated` is
    /// not.
    Token { negated: bool, place: Place, regex: Regex },
    /// Its neighbour at `place` (`Before` or `After`) is a stored child: the token is the first (or the last) of its
    /// gap, and the relation's own walk reads the child (`compile::ast_rule::gap_split`).
    Stored(Place),
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

/// What a gap test found under the token's parent (`owner`): its stored children, the items in
/// order, the implied tokens that pass (`at`, item indices) and, for a neighbour walk, the item it
/// stopped at.
struct Found<'t, 'g, D: Doc> {
    grammar: &'static TokenGrammar,
    owner: Node<'t, D>,
    children: Vec<Node<'t, D>>,
    items: Vec<Item<'g>>,
    at: Vec<usize>,
    neighbour: Option<usize>,
}

impl GapTest {
    /// The token at the test's place that the regex matches, its byte range in the file and its
    /// text: the first implied one, or a stored token where the tape keeps it (the relation's own
    /// walk reads that one). An implied token's range is read from the source between its
    /// parent's stored children: the gap holds only its word, whitespace and extras. `None` where
    /// no token matches.
    pub(in crate::code_rule_dsl) fn token<D: Doc>(&self, node: &Node<'_, D>) -> Option<(std::ops::Range<usize>, String)> {
        let found = self.found_items(node, true)?;
        let stored = |at: &usize| matches!(found.items[*at], Item::Child(child) if !found.children[child].is_named() && self.regex.is_match(&found.children[child].text()));
        let candidate = match &self.place {
            Place::Own | Place::Field(_) => (0..found.items.len()).find(|at| found.at.first() == Some(at) || stored(at)),
            Place::Before | Place::After => found.at.first().copied().or(found.neighbour.filter(stored)),
            Place::Earlier | Place::Later => found.at.first().copied(),
        }?;
        match found.items[candidate] {
            Item::Child(child) => Some((found.children[child].range(), found.children[child].text().into_owned())),
            Item::Token(text, _) => {
                let start = self.token_starts(found.grammar, &found.owner, &found.children, &found.items)[candidate]?;
                Some((start..start + text.len(), text.to_owned()))
            }
        }
    }

    /// The start of each implied token among `items`, by its place in the source: a gap's tokens
    /// lie after the named child before it that is not an extra (or the parent's start), in
    /// order, with only white space, extras and stored tokens between them. `None` for a stored
    /// child, and for a token the source does not hold where the grammar says.
    fn token_starts<D: Doc>(&self, grammar: &TokenGrammar, owner: &Node<'_, D>, children: &[Node<'_, D>], items: &[Item<'_>]) -> Vec<Option<usize>> {
        let passed = |child: &Node<'_, D>| !child.is_named() || grammar.is_extra(&child.kind());
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
    fn interleaved<'g, D: Doc>(&self, grammar: &TokenGrammar, owner: &Node<'_, D>, children: &[Node<'_, D>], items: Vec<Item<'g>>) -> Vec<Item<'g>> {
        let passed = |child: usize| !children[child].is_named() || grammar.is_extra(&children[child].kind());
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
        let starts = self.token_starts(grammar, owner, children, &items);
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

    /// The parent's items in order and what the test finds among them (Tim's reading, 48bf01694 and c00a1d9f0): the
    /// stored neighbour test (`stored`) and a capture's span (`token`) read it; `holds` and `count` read the gap index.
    fn found_items<'t, D: Doc>(&self, node: &Node<'t, D>, first: bool) -> Option<Found<'t, 'static, D>> {
        let grammar = self.grammar()?;
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
        let read = gaps_of(self.dialect, grammar, &owner)?;
        let children: Vec<Node<'t, D>> = owner.children().collect();
        let items = self.interleaved(grammar, &owner, &children, sequence(children.len(), &read.gaps));
        // The token at `at` matches the regex and every condition on it.
        let token_at = |at: usize| match items[at] {
            Item::Token(text, _) => self.regex.is_match(text) && self.token.iter().all(|condition| condition.holds_at(&items, &children, at)),
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
            let at = items.iter().position(|item| matches!(item, Item::Child(child) if children[*child].node_id() == node.node_id()))?;
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
                Place::Before | Place::After => {
                    let place = if self.place == Place::Before { neighbour(Box::new((0..at).rev())) } else { neighbour(Box::new(at + 1..items.len())) };
                    (place.filter(|place| token_at(*place)).into_iter().collect(), place)
                }
                Place::Earlier => (collect(&mut (0..at)), None),
                Place::Later => (collect(&mut (at + 1..items.len())), None),
                Place::Own | Place::Field(_) => unreachable!("a node's own tokens are read above"),
            }
        };
        Some(Found { grammar, owner, children, items, at, neighbour })
    }
}

impl TokenCondition {
    /// Whether the condition holds of the token at `at` among `items`.
    fn holds_at<D: Doc>(&self, items: &[Item<'_>], children: &[Node<'_, D>], at: usize) -> bool {
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
    fn grammar(&self) -> Option<&'static TokenGrammar> {
        TokenGrammar::of_dialect_static(self.dialect)
    }

    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        if self.stored {
            // Where the gaps cannot be read the node keeps its tokens, and the walk reads them.
            let found = self.found_items(node, true);
            return found.is_none_or(|found| found.neighbour.is_none_or(|at| matches!(found.items[at], Item::Child(_))));
        }
        let Some(grammar) = self.grammar() else { return false };
        let matches = |token: &(&str, &str)| self.regex.is_match(token.0);
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
                let Some(parent_gaps) = gaps_of(self.dialect, grammar, node) else { return false };
                let gaps = &parent_gaps.gaps;
                let field = match &self.place {
                    Place::Field(field) => Some(field.as_str()),
                    _ => None,
                };
                // Each word with its place (the end's after the last stored child), so a token's own conditions read
                // the tokens and children beside it (`token_holds`).
                let n = parent_gaps.ids.len();
                let words = gaps.before.iter().take(n).chain(std::iter::once(&gaps.end)).enumerate();
                words.into_iter().any(|(k, word)| {
                    word.iter().enumerate().any(|(i, token)| field.is_none_or(|field| token.1 == field) && matches(token) && self.token_holds(gaps, node, k, i))
                })
            }
            Place::Before | Place::After | Place::Earlier | Place::Later => {
                let Some(parent) = node.parent() else { return false };
                let Some(parent_gaps) = gaps_of(self.dialect, grammar, &parent) else { return false };
                let gaps = &parent_gaps.gaps;
                let Ok(at) = parent_gaps.ids.binary_search(&node.node_id()) else { return false };
                // The stored children as nodes, only where a walk's stop rule reads them.
                let children: Vec<Node<'_, D>> = if past.is_some() { parent.children().collect() } else { Vec::new() };
                // A stored child the walk passes: one its stop rule matches. The gap's word comes
                // before the extras between the two stored children.
                let passed = |k: usize| {
                    use ast_grep_core::matcher::Matcher;
                    past.is_some_and(|stop| stop.match_node_with_env(children[k].clone(), &mut std::borrow::Cow::Owned(ast_grep_core::meta_var::MetaVarEnv::new())).is_some())
                };
                match self.place {
                    // The first token back that the walk does not pass, through the gaps of the
                    // children it passes; a stored child is the relation's own walk's.
                    // An extra right before the node is its neighbour, unless the walk passes it.
                    Place::Before if past.is_none() && at > 0 && parent_gaps.extra[at - 1] => false,
                    Place::Before => {
                        let mut k = at;
                        loop {
                            if let Some(i) = gaps.before[k].iter().rposition(|token| !skipped(token.0)) {
                                return matches(&gaps.before[k][i]) && self.token_holds(gaps, &parent, k, i);
                            }
                            if k == 0 || !passed(k - 1) {
                                return false;
                            }
                            k -= 1;
                        }
                    }
                    // A matching gap at or before the node's, or after it (the end last).
                    Place::Earlier => parent_gaps.matching(self).0 <= at as u32,
                    Place::Later => {
                        let last = parent_gaps.matching(self).1;
                        last != NONE && last > at as u32
                    }
                    // The gap after the node: the first token on that the walk does not pass,
                    // before the next children, or at the end.
                    _ => {
                        let mut k = at + 1;
                        loop {
                            let word = if k < parent_gaps.ids.len() { &gaps.before[k] } else { &gaps.end };
                            if let Some(i) = word.iter().position(|token| !skipped(token.0)) {
                                return matches(&word[i]) && self.token_holds(gaps, &parent, k.min(parent_gaps.ids.len()), i);
                            }
                            if k >= children.len() || !passed(k) {
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

impl GapTest {
    /// Whether the token at `i` of gap `k` (the end where `k` is the stored children's count) of `parent` meets every
    /// condition of `token`: its parent's tokens in order are each gap's, then the stored child after it.
    fn token_holds<D: Doc>(&self, gaps: &Gaps<'static>, parent: &Node<'_, D>, k: usize, i: usize) -> bool {
        if self.token.is_empty() {
            return true;
        }
        let children: Vec<Node<'_, D>> = parent.children().collect();
        let n = children.len();
        let word = |k: usize| if k < n { gaps.before[k].as_slice() } else { gaps.end.as_slice() };
        let child = |k: usize, regex: &Regex| regex.is_match(&children[k].text());
        self.token.iter().all(|condition| {
            let (negated, place, regex) = match condition {
                TokenCondition::Token { negated, place, regex } => (*negated, place, regex),
                TokenCondition::Stored(Place::Before) => return i == 0 && k > 0,
                TokenCondition::Stored(Place::After) => return i + 1 == word(k).len() && k < n,
                TokenCondition::Stored(_) => return false,
            };
            let found = match place {
                // The token right before: earlier in its gap, else the stored child before the gap.
                Place::Before => if i > 0 { regex.is_match(word(k)[i - 1].0) } else { k > 0 && child(k - 1, regex) },
                // Right after: later in its gap, else the stored child after it.
                Place::After => match word(k).get(i + 1) {
                    Some(token) => regex.is_match(token.0),
                    None => k < n && child(k, regex),
                },
                Place::Earlier => {
                    word(k)[..i].iter().any(|token| regex.is_match(token.0))
                        || (0..k).any(|j| child(j, regex) || gaps.before[j].iter().any(|token| regex.is_match(token.0)))
                }
                Place::Later => {
                    word(k)[i + 1..].iter().any(|token| regex.is_match(token.0))
                        || (k..n).any(|j| child(j, regex) || word(j + 1).iter().any(|token| regex.is_match(token.0)))
                }
                Place::Own | Place::Field(_) => return false,
            };
            found != negated
        })
    }

    /// The implied tokens `regex` matches at an `Earlier` or `Later` test's place from `node`, which `count` adds to the
    /// stored nodes it counts (`count.rs`): the tape leaves them out, and no gap holds a stored token. `None` where the
    /// parent's gaps cannot be read (its tokens are then stored, and walked).
    pub(in crate::code_rule_dsl) fn implied_count<D: Doc>(&self, node: &Node<'_, D>) -> Option<usize> {
        let grammar = self.grammar()?;
        let parent = node.parent()?;
        let parent_gaps = gaps_of(self.dialect, grammar, &parent)?;
        let at = parent_gaps.ids.binary_search(&node.node_id()).ok()?;
        let gaps = &parent_gaps.gaps;
        let count = |word: &[(&str, &str)]| word.iter().filter(|token| self.regex.is_match(token.0)).count();
        match self.place {
            Place::Earlier => Some(gaps.before[..=at].iter().map(|word| count(word)).sum()),
            Place::Later => Some(gaps.before[at + 1..].iter().map(|word| count(word)).sum::<usize>() + count(&gaps.end)),
            _ => None,
        }
    }

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
