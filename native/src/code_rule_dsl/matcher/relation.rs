use super::AstMatcher;
use super::CaptureAnalysis;
use super::RequiredText;
use super::Semantic;
use super::memo::Truth;
use super::util::{THREE_VALUED, UNKNOWN};
use crate::code_rule_dsl::budget;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::matcher::{Matcher, MatcherExt, PatternNode};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::sync::{Arc, OnceLock};

/// How far a relation walks, as ast-grep-config 0.45.3's `StopBy` (`rule/stop_by.rs`).
#[derive(Clone)]
pub(crate) enum StopBy {
    Neighbor,
    End,
    /// Walk until the first node this rule matches, inclusive: that node is still tested against the
    /// relation's rule, the walk stops after it. The stop rule matches in a fresh environment
    /// (`Node::matches`), so it neither reads nor binds the caller's metavariables.
    Rule(Box<AstMatcher>),
}

/// Whether a stop rule matches `node`; `None` when its verdict is unknown (a value it reads is).
/// `three` is `THREE_VALUED`, read once by the caller.
pub(super) fn stops_at<D: Doc>(stop: &AstMatcher, node: &Node<'_, D>, three: bool) -> Option<bool> {
    super::each::suspended(|| stops_at_inner(stop, node, three))
}

fn stops_at_inner<D: Doc>(stop: &AstMatcher, node: &Node<'_, D>, three: bool) -> Option<bool> {
    if !three {
        return Some(stop.match_node(node.clone()).is_some());
    }
    let saved = UNKNOWN.replace(false);
    let stopped = stop.match_node(node.clone()).is_some();
    let unknown = UNKNOWN.replace(saved);
    match (stopped, unknown) {
        (true, _) => Some(true),
        (false, false) => Some(false),
        (false, true) => None,
    }
}

/// Whether a rule can bind a capture: a pattern with a named metavariable, through the composites,
/// relations and utilities that share the caller's environment, or a count's value (`as`).
/// `not`, a count's own rule and stop rules bind nothing the caller keeps.
pub(super) fn may_bind(matcher: &AstMatcher) -> bool {
    !super::captures::binds(matcher).is_empty() || !super::captures::counts(matcher).is_empty()
}

/// A relation's search for its first matching node, three-valued. A candidate whose verdict is
/// unknown leaves the answer open unless a later candidate matches; when the relation's rule binds
/// captures, which node binds them first is then unknown too, so the search ends there (strict).
/// An unknown stop verdict is read as a stop that leaves the answer open.
struct Search {
    three: bool,
    /// Whether `violation.each` is excluding bindings (read once per search).
    each: bool,
    saved: bool,
    unknown: bool,
    strict: Option<bool>,
    /// In a two-valued clause, what a candidate needs for the rule to match it: one without is
    /// passed over untested (it could only fail).
    filter: Option<Arc<WalkFilter>>,
}

/// A candidate's test while `violation.each` excludes bindings: one that binds the enumerated
/// capture to a reported range is passed over, bindings and all. `None` where it does not match.
/// Kept out of `Search::test` so that the recursion's frames stay small when nothing is excluded.
#[inline(never)]
fn test_excluding<'t, D: Doc>(
    matcher: &AstMatcher,
    candidate: Node<'t, D>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
) -> Option<ControlFlow<Option<Node<'t, D>>>> {
    let mut trial = Cow::Borrowed(env.as_ref());
    let found = matcher.match_node_with_env(candidate, &mut trial)?;
    if super::each::excluded(&trial) {
        return Some(ControlFlow::Continue(()));
    }
    if let Cow::Owned(bound) = trial {
        *env = Cow::Owned(bound);
    }
    Some(ControlFlow::Break(Some(found)))
}

impl Search {
    fn start(filter: Option<Arc<WalkFilter>>) -> Self {
        let three = THREE_VALUED.get();
        Self {
            three,
            each: super::each::active(),
            saved: three && UNKNOWN.replace(false),
            unknown: false,
            strict: None,
            filter: filter.filter(|_| !three),
        }
    }

    /// Tests one candidate: breaks with the match, or with `None` where a strict search must end
    /// or the deadline passed.
    fn test<'t, D: Doc>(
        &mut self,
        matcher: &AstMatcher,
        candidate: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> ControlFlow<Option<Node<'t, D>>> {
        #[cfg(test)]
        CANDIDATES_TESTED.set(CANDIDATES_TESTED.get() + 1);
        if let Some(filter) = &self.filter
            && filter.kinds.binary_search(&candidate.kind_id()).is_err()
        {
            // A candidate passed over is not matched, which polls the deadline: the walk does.
            if budget::matcher_call_budget_exhausted() {
                return ControlFlow::Break(None);
            }
            return ControlFlow::Continue(());
        }
        if self.each {
            if let Some(flow) = test_excluding(matcher, candidate, env) {
                return flow;
            }
        } else if let Some(found) = matcher.match_node_with_env(candidate, env) {
            return ControlFlow::Break(Some(found));
        }
        if self.three && UNKNOWN.replace(false) {
            return self.open(matcher);
        }
        ControlFlow::Continue(())
    }

    /// Records an unknown verdict; breaks where the search must end (strict).
    fn open<'t, D: Doc>(&mut self, matcher: &AstMatcher) -> ControlFlow<Option<Node<'t, D>>> {
        self.unknown = true;
        if *self.strict.get_or_insert_with(|| may_bind(matcher)) {
            ControlFlow::Break(None)
        } else {
            ControlFlow::Continue(())
        }
    }

    fn finish<'t, D: Doc>(self, found: Option<Node<'t, D>>) -> Option<Node<'t, D>> {
        if !self.three {
            return found;
        }
        match found {
            Some(found) => {
                UNKNOWN.set(self.saved);
                Some(found)
            }
            None => {
                UNKNOWN.set(self.saved || self.unknown);
                None
            }
        }
    }
}

impl StopBy {
    /// Visits the nodes `find` would test, in order, until `visit` breaks. An unknown stop verdict
    /// ends the walk and is reported through `open`.
    pub(super) fn walk<'t, D: Doc, I: Iterator<Item = Node<'t, D>>>(
        &self,
        once: impl FnOnce() -> Option<Node<'t, D>>,
        all: impl FnOnce() -> I,
        mut visit: impl FnMut(Node<'t, D>) -> ControlFlow<()>,
        open: &Cell<bool>,
        three: bool,
    ) -> ControlFlow<()> {
        match self {
            Self::Neighbor => once().map_or(ControlFlow::Continue(()), visit),
            Self::End => all().try_for_each(visit),
            Self::Rule(stop) => {
                for node in all() {
                    let stopped = stops_at(stop, &node, three);
                    visit(node)?;
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            open.set(true);
                            break;
                        }
                    }
                }
                ControlFlow::Continue(())
            }
        }
    }

    /// ast-grep-config's `StopBy::find` for `inside`, `precedes` and `follows`: `once` is the
    /// neighbour, `all` the walk along the axis; with a rule, `all` is cut after the first stop node.
    pub(super) fn find<'t, D: Doc, I: Iterator<Item = Node<'t, D>>>(
        &self,
        once: impl FnOnce() -> Option<Node<'t, D>>,
        all: impl FnOnce() -> I,
        related: &Related,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let matcher = &related.rule;
        let mut search = Search::start(WalkFilter::of(matcher, &related.filter));
        let found = match self {
            Self::Neighbor => once()
                .and_then(|candidate| search.test(matcher, candidate, env).break_value().flatten()),
            Self::End => all()
                .try_for_each(|candidate| search.test(matcher, candidate, env))
                .break_value()
                .flatten(),
            Self::Rule(stop) => {
                let mut found = None;
                for candidate in all() {
                    let stopped = stops_at(stop, &candidate, search.three);
                    if let ControlFlow::Break(result) = search.test(matcher, candidate, env) {
                        found = result;
                        break;
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            let _ = search.open::<D>(matcher);
                            break;
                        }
                    }
                }
                found
            }
        };
        search.finish(found)
    }

    /// `find` as a verdict, for `inside`, `precedes` and `follows` from `node`. To the end, a
    /// node-local rule (`Related::chains`) matches at a node or past it for every walk that passes
    /// it alike: outside `violation.each` and a `parse`'s host, that is remembered for the file
    /// (`reach_end`), so each node is tested once per relation rather than once per walk.
    pub(super) fn reaches<'t, D: Doc, I: Iterator<Item = Node<'t, D>>>(
        &self,
        node: &Node<'t, D>,
        once: impl FnOnce() -> Option<Node<'t, D>>,
        all: impl FnOnce() -> I,
        related: &Related,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> bool {
        if matches!(self, Self::End)
            && related.remembers()
            && super::memo::file_node(node).is_some()
        {
            let walk = all().filter_map(|candidate| super::memo::file_node(&candidate).cloned());
            return settle(reach_end(walk, related));
        }
        self.find(once, all, related, env).is_some()
    }

    /// `reaches` for `precedes` (`forward`) and `follows`. To the end, a rule looking for a
    /// sibling with the same key as a capture bound outside it walks only the siblings it may
    /// match (`Related::same_key_siblings`).
    pub(super) fn find_sibling<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        forward: bool,
        related: &Related,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> bool {
        if matches!(self, Self::End)
            && let Some(listed) = related.same_key_siblings(node, env, forward)
        {
            return self
                .find(|| None, || listed.into_iter(), related, env)
                .is_some();
        }
        if forward {
            self.reaches(node, || node.next(), || node.next_all(), related, env)
        } else {
            self.reaches(node, || node.prev(), || node.prev_all(), related, env)
        }
    }
}

/// One candidate's verdict for a node-local rule (`Related::chains`), which reads no binding, and
/// whether testing it spent a utility step (a value decoded). `filter` passes over candidates the
/// rule cannot match, as `Search` does. `None` where the deadline passed or a limit was hit.
fn verdict(
    related: &Related,
    filter: Option<&WalkFilter>,
    candidate: Node<'_, TapeDoc>,
    three: bool,
) -> Option<(Truth, bool)> {
    #[cfg(test)]
    CANDIDATES_TESTED.set(CANDIDATES_TESTED.get() + 1);
    if let Some(filter) = filter
        && filter.kinds.binary_search(&candidate.kind_id()).is_err()
    {
        return (!budget::matcher_call_budget_exhausted()).then_some((Truth::False, false));
    }
    let steps = super::utility_steps_left();
    let mut env = Cow::Owned(MetaVarEnv::new());
    let matched = related
        .rule
        .match_node_with_env(candidate, &mut env)
        .is_some();
    let unknown = three && UNKNOWN.replace(false);
    if budget::matcher_budget_observed() || super::util_limit_hit() {
        return None;
    }
    let truth = if matched {
        Truth::True
    } else if unknown {
        Truth::Unknown
    } else {
        Truth::False
    };
    Some((truth, super::utility_steps_left() != steps))
}

/// A remembered relation's answer as the matcher's verdict: an unknown one sets `UNKNOWN` in a
/// three-valued clause, and leaves it as it was otherwise.
fn settle(answer: Truth) -> bool {
    if answer == Truth::Unknown && THREE_VALUED.get() {
        UNKNOWN.set(true);
    }
    answer == Truth::True
}

/// Visits descendants in pre-order; a descendant `stop` matches is visited but not entered. One
/// whose stop verdict is unknown is not entered either, and sets `open`.
pub(super) fn descend_until<'tree, D: Doc>(
    node: &Node<'tree, D>,
    stop: &AstMatcher,
    mut visit: impl FnMut(Node<'tree, D>) -> ControlFlow<()>,
    open: &Cell<bool>,
    three: bool,
) -> ControlFlow<()> {
    let level = |node: &Node<'tree, D>| node.children().collect::<Vec<_>>().into_iter();
    let mut stack = vec![level(node)];
    while let Some(children) = stack.last_mut() {
        let Some(child) = children.next() else {
            stack.pop();
            continue;
        };
        visit(child.clone())?;
        match stops_at(stop, &child, three) {
            Some(false) => stack.push(level(&child)),
            Some(true) => {}
            None => open.set(true),
        }
    }
    ControlFlow::Continue(())
}

/// `StopBy::reaches` to the end over the evaluated file's nodes, for a node-local rule: the walk
/// stops at the first node that matches or whose answer the file remembers, and every node it
/// passed has the `or` of its own verdict and the answer past it. An answer is remembered only
/// where no node from it on spent a utility step to be tested (a value decoded): walking again
/// would spend none either, so a remembered answer has no steps to replay. Nothing is remembered
/// from a walk the deadline or a limit cut short.
#[inline(never)]
fn reach_end<'t>(all: impl Iterator<Item = Node<'t, TapeDoc>>, related: &Related) -> Truth {
    let three = THREE_VALUED.get();
    let saved = three && UNKNOWN.replace(false);
    let filter = WalkFilter::of(&related.rule, &related.filter).filter(|_| !three);
    let term = std::ptr::from_ref(related) as usize;
    let mut walked = Vec::new();
    let mut answer = Truth::False;
    for candidate in all {
        let id = candidate.node_id();
        if let Some(known) = super::memo::answer(term, id, three) {
            answer = known;
            break;
        }
        let Some((own, spent)) = verdict(related, filter.as_deref(), candidate, three) else {
            UNKNOWN.set(saved);
            return Truth::False;
        };
        walked.push((id, own, spent));
        if own == Truth::True {
            answer = Truth::True;
            break;
        }
    }
    let mut clean = true;
    let mut remembered = Vec::new();
    for &(id, own, spent) in walked.iter().rev() {
        answer = answer.max(own);
        clean &= !spent;
        if clean {
            remembered.push((id, answer));
        }
    }
    super::memo::remember_answers(term, three, &remembered);
    UNKNOWN.set(saved);
    answer
}

/// `has`: whether a child (`neighbor`) or descendant in pre-order matches; with a stop rule, a
/// descendant the stop rule matches is tested but its subtree is not entered.
pub(super) fn has<'tree, D: Doc>(
    node: &Node<'tree, D>,
    related: &Related,
    stop: &StopBy,
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> bool {
    let matcher = &related.rule;
    let filter = WalkFilter::of(matcher, &related.filter);
    // A two-valued walk a `WalkFilter` bounds tests the postings alone (`memo::bounded`).
    if matches!(stop, StopBy::End)
        && (THREE_VALUED.get() || filter.is_none())
        && related.remembers()
        && let Some(node) = super::memo::file_node(node)
    {
        return settle(has_remembered(node, related));
    }
    let mut search = Search::start(filter.clone());
    let found = match stop {
        StopBy::Neighbor => node
            .children()
            .try_for_each(|child| search.test(matcher, child, env))
            .break_value()
            .flatten(),
        StopBy::End => has_descendant(node, matcher, filter.as_ref(), &mut search, env),
        StopBy::Rule(stop) => has_until(node, matcher, stop, &mut search, env),
    };
    search.finish(found).is_some()
}

/// `has` to the end of a node-local rule (`Related::chains`) at the evaluated file's `node`,
/// remembered for the file. The walk goes through the subtree in pre-order, passing over subtrees
/// whose answer is known. A match proves the answer for every node from the match's parent up to
/// `node`; otherwise each node walked has the `or` of its children's answers and verdicts,
/// computed bottom-up. Answers are remembered as `reach_end` remembers them: only where no node
/// below the one answered for spent a utility step to be tested, and never from a walk the
/// deadline or a limit cut short.
#[inline(never)]
fn has_remembered(node: &Node<'_, TapeDoc>, related: &Related) -> Truth {
    let three = THREE_VALUED.get();
    let term = std::ptr::from_ref(related) as usize;
    let top = node.node_id();
    if let Some(known) = super::memo::answer(term, top, three) {
        return known;
    }
    let saved = three && UNKNOWN.replace(false);
    let root = node.root();
    let tape = root.root().get_doc();
    let range = tape.transducer_descendants(top as u32);
    // Each node walked, with its parent, its own verdict, its remembered answer and whether
    // testing it spent a utility step.
    let mut walked: Vec<(u32, u32, Truth, Option<Truth>, bool)> = Vec::new();
    // The nodes entered whose subtree the walk is in, with where each subtree ends.
    let mut open: Vec<(u32, u32)> = Vec::new();
    let mut last_spent = None;
    let mut matched = false;
    let mut candidate = range.start;
    while candidate < range.end {
        while open.last().is_some_and(|&(_, end)| end <= candidate) {
            open.pop();
        }
        let parent = open.last().map_or(top as u32, |&(entered, _)| entered);
        let Some((own, spent)) = verdict(related, None, root.adopt(tape.node(candidate)), three)
        else {
            UNKNOWN.set(saved);
            return Truth::False;
        };
        if spent {
            last_spent = Some(candidate);
        }
        if own == Truth::True {
            matched = true;
            break;
        }
        let known = super::memo::answer(term, candidate as usize, three);
        walked.push((candidate, parent, own, known, spent));
        match known {
            Some(Truth::True) => {
                matched = true;
                break;
            }
            Some(_) => candidate = tape.transducer_descendants(candidate).end,
            None => {
                open.push((candidate, tape.transducer_descendants(candidate).end));
                candidate += 1;
            }
        }
    }
    UNKNOWN.set(saved);
    let mut remembered = Vec::new();
    if matched {
        // The match proves every entered node's answer, which reads the nodes walked below it
        // up to the match.
        for (entered, _) in open {
            if last_spent.is_none_or(|spent| spent <= entered) {
                remembered.push((entered as usize, Truth::True));
            }
        }
        if last_spent.is_none() {
            remembered.push((top, Truth::True));
        }
        super::memo::remember_answers(term, three, &remembered);
        return Truth::True;
    }
    // Each entered node's answer so far, and whether testing a node below it spent a step.
    let mut below = rustc_hash::FxHashMap::<u32, (Truth, bool)>::default();
    let (mut answer, mut dirty) = (Truth::False, false);
    for &(candidate, parent, own, known, spent) in walked.iter().rev() {
        let (under, spent_under) = match known {
            Some(known) => (known, false),
            None => {
                let (under, spent_under) =
                    below.remove(&candidate).unwrap_or((Truth::False, false));
                if !spent_under {
                    remembered.push((candidate as usize, under));
                }
                (under, spent_under)
            }
        };
        let (value, spent) = (own.max(under), spent || spent_under);
        if parent == top as u32 {
            answer = answer.max(value);
            dirty |= spent;
        } else {
            let slot = below.entry(parent).or_insert((Truth::False, false));
            slot.0 = slot.0.max(value);
            slot.1 |= spent;
        }
    }
    if !dirty {
        remembered.push((top, answer));
    }
    super::memo::remember_answers(term, three, &remembered);
    answer
}

/// `has` to the end: the descendants in pre-order (those `filter` bounds, where it can).
/// Out of line, as every arm's locals would otherwise share the recursion's frame.
#[inline(never)]
fn has_descendant<'tree, D: Doc>(
    node: &Node<'tree, D>,
    matcher: &AstMatcher,
    filter: Option<&Arc<WalkFilter>>,
    search: &mut Search,
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<Node<'tree, D>> {
    match filter.and_then(|filter| {
        super::memo::bounded(filter, node, |tape, ordinal| {
            tape.transducer_descendants(ordinal)
        })
    }) {
        Some(candidates) => candidates
            .into_iter()
            .try_for_each(|child| search.test(matcher, child, env))
            .break_value()
            .flatten(),
        None => node
            .dfs()
            .skip(1)
            .try_for_each(|child| search.test(matcher, child, env))
            .break_value()
            .flatten(),
    }
}

/// `has` with a stop rule (see `descend_until`).
#[inline(never)]
fn has_until<'tree, D: Doc>(
    node: &Node<'tree, D>,
    matcher: &AstMatcher,
    stop: &AstMatcher,
    search: &mut Search,
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<Node<'tree, D>> {
    let mut found = None;
    let open = Cell::new(false);
    let three = search.three;
    let _ = descend_until(
        node,
        stop,
        |child| {
            if budget::matcher_call_budget_exhausted() {
                return ControlFlow::Break(());
            }
            match search.test(matcher, child, env) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(result) => {
                    found = result;
                    return ControlFlow::Break(());
                }
            }
            // A subtree left unentered on an unknown stop verdict leaves the answer open.
            if open.take() && search.open::<D>(matcher).is_break() {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        },
        &open,
        three,
    );
    if open.get() {
        let _ = search.open::<D>(matcher);
    }
    found
}

/// A relation's rule (`inside`, `has`, `precedes`, `follows`), with what a node needs for it to
/// match (`WalkFilter`), whether its verdict is the node's alone (`chains`) and whether it looks
/// for a sibling with the same key as a capture bound outside it (`SameKey`), worked out on first
/// use.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Related {
    rule: AstMatcher,
    filter: OnceLock<Option<Arc<WalkFilter>>>,
    chains: OnceLock<bool>,
    same_key: OnceLock<Option<SameKey>>,
}

impl Related {
    pub(in crate::code_rule_dsl) fn new(rule: AstMatcher) -> Box<Self> {
        Box::new(Self {
            rule,
            filter: OnceLock::new(),
            chains: OnceLock::new(),
            same_key: OnceLock::new(),
        })
    }

    pub(in crate::code_rule_dsl) fn rule(&self) -> &AstMatcher {
        &self.rule
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::code_rule_dsl) fn with_rule(&self, rule: AstMatcher) -> Box<Self> {
        Self::new(rule)
    }

    /// Whether the rule's verdict at a node is the node's alone: it mentions no capture and
    /// reads nothing beyond the node's tree and values (`node_local`).
    fn chains(&self) -> bool {
        *self.chains.get_or_init(|| {
            node_local(&self.rule) && CaptureAnalysis::default().mentions(&self.rule).is_empty()
        })
    }

    /// Whether the rule's answers are remembered for the file in this walk: it is node-local
    /// (`chains`), and neither `violation.each` nor a `parse`'s host is being matched.
    fn remembers(&self) -> bool {
        #[cfg(test)]
        if ANSWERS_OFF.get() {
            return false;
        }
        self.chains() && !super::each::active() && !super::parse::hosting()
    }

    /// The siblings of `node` along a walk to the end (`forward`: `next_all`, else `prev_all`)
    /// that the rule may match where it looks for one with the same key as a capture bound outside
    /// it (`SameKey`): those of that capture's class, or of an unknown one, in walk order. `None`
    /// where every sibling is walked: the rule is no such search, the capture is bound to no node,
    /// the rule's own capture is bound already (its pattern then compares texts), a `parse`'s
    /// host is being matched, or `node` is not the evaluated file's.
    pub(super) fn same_key_siblings<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &MetaVarEnv<'t, D>,
        forward: bool,
    ) -> Option<Vec<Node<'t, D>>> {
        #[cfg(test)]
        if SAME_KEY_OFF.get() {
            return None;
        }
        let plan = self
            .same_key
            .get_or_init(|| SameKey::of(&self.rule))
            .as_ref()?;
        if super::parse::hosting()
            || env.get_match(&plan.inside).is_some()
            || !env.get_multiple_matches(&plan.inside).is_empty()
            || env.get_transformed(&plan.inside).is_some()
        {
            return None;
        }
        let outside = env.get_match(&plan.outside)?;
        let AstMatcher::All(terms) = &self.rule else {
            return None;
        };
        let AstMatcher::Compare(comparison) = &terms[plan.at] else {
            return None;
        };
        let class = comparison.eq_class(outside);
        super::memo::same_key_siblings(comparison, node, &class, forward)
    }
}

/// A sibling walk's rule looking for a node with the same key as a capture bound outside it: an
/// `all` holding `pattern: $Q` and, after it, `compare: {left: Q, op: eq, right: P}` (either way
/// round) under a text mode whose verdict each side's class decides (`Comparison::eq_class`),
/// none of its other terms binding `Q` or `P`. A sibling whose class is neither `P`'s nor unknown
/// then fails the comparison, which nothing unknown can withhold (only the pattern binds what it
/// reads), and a false term fails an `all`: the sibling neither matches nor leaves the verdict
/// unknown, so a walk may pass over it untested.
#[derive(Clone)]
struct SameKey {
    /// The comparison's index in the `all`.
    at: usize,
    /// `Q`, which the pattern binds to the sibling.
    inside: String,
    /// `P`, bound outside the rule.
    outside: String,
}

impl SameKey {
    fn of(rule: &AstMatcher) -> Option<Self> {
        let AstMatcher::All(terms) = rule else {
            return None;
        };
        let mut captures = CaptureAnalysis::default();
        for (at, term) in terms.iter().enumerate() {
            let AstMatcher::Compare(comparison) = term else {
                continue;
            };
            if !comparison.classes_decide() {
                continue;
            }
            for (inside, outside) in [
                (&comparison.left, &comparison.right),
                (&comparison.right, &comparison.left),
            ] {
                let Some(pattern) = terms[..at]
                    .iter()
                    .position(|term| lone_capture(term) == Some(inside.as_str()))
                else {
                    continue;
                };
                let others_bind = terms
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| index != at && index != pattern)
                    .any(|(_, term)| {
                        let binds = captures.binds(term);
                        binds.contains(inside) || binds.contains(outside)
                    });
                if inside == outside || others_bind {
                    continue;
                }
                return Some(Self {
                    at,
                    inside: inside.clone(),
                    outside: outside.clone(),
                });
            }
        }
        None
    }
}

/// The capture a pattern that is one metavariable binds: the node it is matched at.
fn lone_capture(term: &AstMatcher) -> Option<&str> {
    match term {
        AstMatcher::Pattern(pattern) => match &pattern.node {
            PatternNode::MetaVar {
                meta_var: MetaVariable::Capture(name, _),
            } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Whether matching `matcher` reads nothing but the node's tree and values, and spends no utility
/// step but those decoding a value: patterns, kinds, regexes and `value`, through composites,
/// relations, constraints and `nthChild` without `ofRule`. A value reads the node and the file's
/// bindings, never the environment.
fn node_local(matcher: &AstMatcher) -> bool {
    let stop_local = |stop: &StopBy| stop_rule(stop).is_none_or(node_local);
    match matcher {
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_) | AstMatcher::Supertype(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Never
        | AstMatcher::Semantic(Semantic::Value(_)) => true,
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().all(node_local),
        AstMatcher::Not(inner) => node_local(inner),
        AstMatcher::Inside(related, stop)
        | AstMatcher::Has(related, stop)
        | AstMatcher::Precedes(related, stop)
        | AstMatcher::Follows(related, stop) => node_local(&related.rule) && stop_local(stop),
        AstMatcher::Field(relation) => node_local(&relation.rule) && stop_local(&relation.stop),
        AstMatcher::NthChild(nth) => nth.of_rule.is_none(),
        AstMatcher::Constrained(core) => {
            node_local(&core.rule)
                && core
                    .constraints
                    .iter()
                    .all(|(_, constraint)| node_local(constraint))
        }
        AstMatcher::Util(_)
        | AstMatcher::Call(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Count(_)
        | AstMatcher::Semantic(_) => false,
        #[cfg(any(test, feature = "test-hooks"))]
        AstMatcher::TestUnknown(_) => false,
    }
}

impl std::ops::Deref for Related {
    type Target = AstMatcher;

    fn deref(&self) -> &AstMatcher {
        &self.rule
    }
}

/// What a node needs for a relation's rule to match it, where that is known: one of `kinds`
/// (sorted), and text holding `needle`. A walk down the tree tests only such nodes
/// (`memo::bounded`): its rule fails at every other.
pub(crate) struct WalkFilter {
    pub(super) kinds: Arc<[u16]>,
    pub(super) needle: Option<RequiredText>,
}

impl WalkFilter {
    fn of(rule: &AstMatcher, cell: &OnceLock<Option<Arc<Self>>>) -> Option<Arc<Self>> {
        cell.get_or_init(|| {
            let kinds = node_kinds(rule, &mut BTreeMap::new())?;
            Some(Arc::new(Self {
                kinds: kinds.into_iter().collect(),
                needle: RequiredText::of_node(rule),
            }))
        })
        .clone()
    }
}

/// The kinds every node `matcher` can match has, where it says (`None`: any kind). Utilities read
/// so far are in `utilities` (`None` while one is read: a recursive reference allows any kind).
fn node_kinds(
    matcher: &AstMatcher,
    utilities: &mut BTreeMap<(usize, usize), Option<BTreeSet<u16>>>,
) -> Option<BTreeSet<u16>> {
    let util_kinds =
        |util: &super::UtilRef, utilities: &mut BTreeMap<(usize, usize), Option<BTreeSet<u16>>>| {
            let key = (util.table.as_ptr() as usize, util.index);
            if let Some(known) = utilities.get(&key) {
                return known.clone();
            }
            utilities.insert(key, None);
            let kinds = util
                .with_target(|rule| node_kinds(rule, utilities))
                .flatten();
            utilities.insert(key, kinds.clone());
            kinds
        };
    match matcher {
        AstMatcher::Supertype(supertype) => Some(supertype.kinds.iter().copied().collect()),
        AstMatcher::Kind(kind) => kind
            .potential_kinds()
            .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
        AstMatcher::Pattern(pattern) => pattern
            .potential_kinds()
            .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
        AstMatcher::Never => Some(BTreeSet::new()),
        AstMatcher::All(items) => items
            .iter()
            .filter_map(|item| node_kinds(item, utilities))
            .reduce(|left, right| &left & &right),
        AstMatcher::Any(items) => {
            let mut kinds = BTreeSet::new();
            for item in items {
                kinds.extend(node_kinds(item, utilities)?);
            }
            Some(kinds)
        }
        AstMatcher::Constrained(core) => node_kinds(&core.rule, utilities),
        // A double negation matches where its rule does.
        AstMatcher::Not(inner) => match &**inner {
            AstMatcher::Not(rule) => node_kinds(rule, utilities),
            _ => None,
        },
        AstMatcher::Util(util) => util_kinds(util, utilities),
        AstMatcher::Call(call) => util_kinds(&call.util, utilities),
        _ => None,
    }
}

/// `inside` or `has` with a `field` (ast-grep-config 0.45.3 `Inside`/`Has`, `relational_rule.rs`).
#[derive(Clone)]
pub(crate) struct FieldRelation {
    /// `inside` when true, `has` otherwise.
    pub(crate) inside: bool,
    /// The grammar's field id.
    pub(crate) field: u16,
    pub(crate) rule: AstMatcher,
    pub(crate) stop: StopBy,
    /// What a node needs for `rule` to match it (`WalkFilter`), worked out on first use.
    pub(crate) filter: OnceLock<Option<Arc<WalkFilter>>>,
}

impl FieldRelation {
    pub(super) fn find<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        if self.inside {
            self.find_inside(node, env)
        } else {
            self.find_has(node, env)
        }
    }

    /// The ancestors walked as `stopBy` walks them; each is tested only where the node the walk
    /// came from is its child in the field, else skipped (the walk goes on).
    fn find_inside<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let mut search = Search::start(WalkFilter::of(&self.rule, &self.filter));
        let mut last = node.clone();
        let mut entered = |candidate: &Node<'t, D>| {
            let expected = std::mem::replace(&mut last, candidate.clone());
            if let Some(tape) = super::memo::file_node(&expected) {
                let found = tape.get_inner_node().is_first_field(self.field);
                #[cfg(feature = "code-rule-profile")]
                crate::code_rule_dsl::tape::navigation::record_edge(found);
                return found;
            }
            candidate
                .child_by_field_id(self.field)
                .is_some_and(|child| child.node_id() == expected.node_id())
        };
        let found = match &self.stop {
            StopBy::Neighbor => node.parent().and_then(|candidate| {
                if entered(&candidate) {
                    search
                        .test(&self.rule, candidate, env)
                        .break_value()
                        .flatten()
                } else {
                    None
                }
            }),
            StopBy::End => node
                .ancestors()
                .try_for_each(|candidate| {
                    if entered(&candidate) {
                        search.test(&self.rule, candidate, env)
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .break_value()
                .flatten(),
            StopBy::Rule(stop) => {
                let mut found = None;
                for candidate in node.ancestors() {
                    let stopped = stops_at(stop, &candidate, search.three);
                    if entered(&candidate)
                        && let ControlFlow::Break(result) = search.test(&self.rule, candidate, env)
                    {
                        found = result;
                        break;
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            let _ = search.open::<D>(&self.rule);
                            break;
                        }
                    }
                }
                found
            }
        };
        search.finish(found)
    }

    /// The field child, then as `stopBy` says: nothing more (`neighbor`), its descendants in
    /// pre-order (`end`), or, where it neither matches nor stops, its children only (a stop rule;
    /// upstream tests one level below the field child and does not recurse).
    fn find_has<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let child = node.child_by_field_id(self.field)?;
        let filter = WalkFilter::of(&self.rule, &self.filter);
        let mut search = Search::start(filter.clone());
        let found = match &self.stop {
            StopBy::Neighbor => search.test(&self.rule, child, env).break_value().flatten(),
            StopBy::End => match filter.and_then(|filter| {
                super::memo::bounded(&filter, &child, |tape, ordinal| {
                    ordinal..tape.transducer_descendants(ordinal).end
                })
            }) {
                Some(candidates) => candidates
                    .into_iter()
                    .try_for_each(|candidate| search.test(&self.rule, candidate, env))
                    .break_value()
                    .flatten(),
                None => child
                    .dfs()
                    .try_for_each(|candidate| search.test(&self.rule, candidate, env))
                    .break_value()
                    .flatten(),
            },
            StopBy::Rule(stop) => match search.test(&self.rule, child.clone(), env) {
                ControlFlow::Break(result) => result,
                ControlFlow::Continue(()) => match stops_at(stop, &child, search.three) {
                    Some(true) => None,
                    None => {
                        let _ = search.open::<D>(&self.rule);
                        None
                    }
                    Some(false) => child
                        .children()
                        .try_for_each(|candidate| search.test(&self.rule, candidate, env))
                        .break_value()
                        .flatten(),
                },
            },
        };
        search.finish(found)
    }
}

pub(crate) fn stop_rule(stop: &StopBy) -> Option<&AstMatcher> {
    match stop {
        StopBy::Rule(rule) => Some(rule),
        StopBy::Neighbor | StopBy::End => None,
    }
}

#[cfg(test)]
thread_local! {
    static CANDIDATES_TESTED: Cell<usize> = const { Cell::new(0) };
    static SAME_KEY_OFF: Cell<bool> = const { Cell::new(false) };
    static ANSWERS_OFF: Cell<bool> = const { Cell::new(false) };
}

/// Walks every sibling where a rule looks for one with the same key (`SameKey`), on this thread.
#[cfg(test)]
pub(crate) fn set_same_key_off(off: bool) {
    SAME_KEY_OFF.set(off);
}

/// Remembers no node-local relation's answers (`Related::remembers`), on this thread.
#[cfg(test)]
pub(crate) fn set_answers_off(off: bool) {
    ANSWERS_OFF.set(off);
}

/// Candidates relations tested on this thread (`reset_candidates_tested` restarts the count).
#[cfg(test)]
fn candidates_tested() -> usize {
    CANDIDATES_TESTED.get()
}

#[cfg(test)]
fn reset_candidates_tested() {
    CANDIDATES_TESTED.set(0);
}

#[cfg(test)]
mod tests;
