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
//! rewritten to ask the gap test and the structural test apart.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::tape::tokens::TokenGrammar;
pub use crate::code_rule_dsl::tape::tokens::stored_gaps;
use ast_grep_core::{Doc, Node};
use regex::Regex;
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
}

/// One condition on the token a gap test found (`GapTest::token`): whether a token `regex` matches is at `place` from
/// it (`Before`, `After`, `Earlier` or `Later` along the parent's tokens), or with `negated` is not.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct TokenCondition {
    pub(in crate::code_rule_dsl) negated: bool,
    pub(in crate::code_rule_dsl) place: Place,
    pub(in crate::code_rule_dsl) regex: Regex,
}

impl std::fmt::Debug for GapTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GapTest").field("place", &self.place).field("regex", &self.regex.as_str()).finish()
    }
}

impl GapTest {
    fn grammar(&self) -> Option<&'static TokenGrammar> {
        TokenGrammar::of_dialect_static(self.dialect)
    }

    /// Whether an implied token the regex matches is at the test's place.
    pub(in crate::code_rule_dsl) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
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
                gaps.before.iter().flatten().chain(&gaps.end).any(|token| field.is_none_or(|field| token.1 == field) && matches(token))
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
            let regex = &condition.regex;
            let found = match condition.place {
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
            found != condition.negated
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
