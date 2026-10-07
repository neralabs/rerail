//! Utility verdicts remembered while one file is evaluated (`with_file`). A `matches` without
//! parameters answers the same at a node for every caller that agrees on the captures it may read
//! or bind (`UtilRef::scope`; every capture where it holds a term that reads more), on what a
//! `finding.each` enumeration excludes there (`each::exclusion_key`), on the binding sub-rules
//! being matched and on whether its check is three-valued; a parameterized call, on the caller's
//! bindings of the captures it maps to its parameters. Its verdict is remembered by those alone: a
//! later caller gets the bindings it added, not the ones it had. Without this, utilities that reach
//! each other through relations would be evaluated again for every caller, without bound but the
//! budget.
//!
//! A remembered verdict replays the utility steps computing it spent (bar the caches it filled,
//! which are read for free again), and is reused only where the utility levels it went down still
//! fit under the depth limit: the budget and the depth limit run out where they would have. A
//! computation that read state kept for its check alone (`read_check_state`: a cache keyed by one
//! of its terms, a file table charged to each check once) would spend differently in another
//! check, so its verdict is that check's alone; the others are every check's.
//! Nothing is remembered from a computation that hit a limit or began a check of its own (a
//! `calls` seed), nor where the verdict reads state the key does not hold: an unknown verdict
//! pending, or a `parse`'s host being matched.

use super::nth_child::Siblings;
use super::relation::WalkFilter;
use super::util::{THREE_VALUED, UNKNOWN, UtilCall, UtilRef};
use super::{Comparison, EqClass};
use super::{
    check_state_reads, checks_begun, each, fill_steps, filling, read_check_state, spend_util_steps,
    swap_util_peak, util_level, util_levels_fit, util_limit_hit, utility_steps_left,
};
use crate::policy_dsl::budget;
use crate::policy_dsl::tape::TapeDoc;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{AstGrep as Root, Doc, Node};
use std::any::TypeId;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

/// One capture's binding, by node ordinal.
#[derive(Clone, Eq, Hash, PartialEq)]
enum Bound {
    Single(u32),
    Multi(Box<[u32]>),
    Text(Box<[u8]>),
}

/// Bindings sorted by name, then kind.
type Bindings = Vec<(Arc<str>, Bound)>;

#[derive(Eq, Hash, PartialEq)]
struct Key {
    /// A utility's `UtilRef::memo_term`, or a call's address and `usize::MAX`.
    term: (usize, usize),
    node: u32,
    three: bool,
    exclusion: (u64, u64),
    /// The binding sub-rules being matched (`binding::following_key`), which a verdict reads.
    following: Vec<(usize, usize)>,
    input: Bindings,
    /// The check whose state computing the verdict read (`resolve::CURRENT_CHECK`); `0` where it
    /// read none, and every check shares it.
    owner: usize,
}

#[derive(Clone)]
enum Verdict {
    /// It matched, adding these bindings.
    Match(Bindings),
    NoMatch,
    Unknown,
}

struct Entry {
    verdict: Verdict,
    steps: usize,
    levels: u32,
}

#[derive(Default)]
struct FileMemo {
    /// The address of the file's `Root`; `0` when no file is being evaluated.
    root: usize,
    /// Verdicts that read no `finding.each` exclusion (`Key::exclusion` zero).
    entries: rustc_hash::FxHashMap<Key, Entry>,
    /// Verdicts read under the exclusion generation `generation` alone: a generation once past
    /// never returns, so those of an earlier one are dropped.
    excluding: rustc_hash::FxHashMap<Key, Entry>,
    generation: u64,
    /// The file's nodes by kind, in preorder, listed on first use.
    kinds: Option<rustc_hash::FxHashMap<u16, Vec<u32>>>,
    /// The nodes passing each walk filter (by its address), in preorder.
    postings: rustc_hash::FxHashMap<usize, Arc<[u32]>>,
    /// `nthChild`'s counted siblings by counting term and parent (`siblings`).
    siblings: rustc_hash::FxHashMap<(usize, usize), Arc<Siblings>>,
    /// A node-local relation's answer to the end (`Related::chains`), by relation, node and
    /// whether its check is three-valued: for `has`, over the node's descendants; for the others,
    /// at the node or past it along the relation's walk (`answer`).
    answers: rustc_hash::FxHashMap<(usize, usize, bool), Truth>,
    /// A parent's children by their class under an `eq` comparison, by comparison and parent
    /// (`same_key_siblings`).
    same_keys: rustc_hash::FxHashMap<(usize, usize), Arc<SameKeys>>,
}

impl FileMemo {
    fn entry(&self, key: &Key) -> Option<&Entry> {
        match key.exclusion.0 {
            // An enumeration's generations start at one.
            0 => self.entries.get(key),
            generation if generation == self.generation => self.excluding.get(key),
            _ => None,
        }
    }

    fn insert(&mut self, key: Key, entry: Entry) {
        match key.exclusion.0 {
            0 => {
                self.entries.insert(key, entry);
            }
            generation => {
                if generation != self.generation {
                    self.excluding.clear();
                    self.generation = generation;
                }
                self.excluding.insert(key, entry);
            }
        }
        #[cfg(test)]
        REMEMBERED_PEAK.set(
            REMEMBERED_PEAK
                .get()
                .max(self.entries.len() + self.excluding.len()),
        );
    }
}

thread_local! {
    static MEMO: RefCell<FileMemo> = RefCell::new(FileMemo::default());
}

#[cfg(test)]
thread_local! {
    static REMEMBERED_PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most verdicts a file's memo held at once on this thread (`reset_remembered_peak` restarts
/// it).
#[cfg(test)]
fn remembered_peak() -> usize {
    REMEMBERED_PEAK.get()
}

#[cfg(test)]
fn reset_remembered_peak() {
    REMEMBERED_PEAK.set(0);
}

/// Restores the enclosing file's memo when a nested one ends.
struct Scoped(Option<FileMemo>);

impl Drop for Scoped {
    fn drop(&mut self) {
        if let Some(saved) = self.0.take() {
            MEMO.set(saved);
        }
    }
}

/// Runs `run` remembering utility verdicts at `root`'s nodes, forgotten when it returns.
pub(in crate::policy_dsl) fn with_file<T>(root: &Root<TapeDoc>, run: impl FnOnce() -> T) -> T {
    let file = FileMemo {
        root: std::ptr::from_ref(root) as usize,
        ..FileMemo::default()
    };
    let _scoped = Scoped(Some(MEMO.replace(file)));
    run()
}

/// `value` as `U`. SAFETY: callers prove `T` and `U` one type by their `TypeId`s first.
unsafe fn same<T, U>(value: T) -> U {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<U>());
    let value = std::mem::ManuallyDrop::new(value);
    // SAFETY: `T` is `U` (the caller's contract); the original is not dropped.
    unsafe { std::mem::transmute_copy(&*value) }
}

/// Nodes listed between two reads of the deadline while a file's index or postings are built.
const POLL_INTERVAL: usize = 256;

/// A file's nodes by kind, in preorder; `None` where the deadline passed first.
fn kind_index(tape: &TapeDoc) -> Option<rustc_hash::FxHashMap<u16, Vec<u32>>> {
    let mut kinds = rustc_hash::FxHashMap::<u16, Vec<u32>>::default();
    for ordinal in 0..tape.transducer_len() as u32 {
        if (ordinal as usize).is_multiple_of(POLL_INTERVAL) && budget::matcher_budget_exhausted() {
            return None;
        }
        kinds
            .entry(tape.transducer_kind(ordinal))
            .or_default()
            .push(ordinal);
    }
    Some(kinds)
}

/// The nodes of `filter`'s kinds whose text holds its needle, in preorder; `None` where the
/// deadline passed first.
fn postings(
    filter: &WalkFilter,
    index: &rustc_hash::FxHashMap<u16, Vec<u32>>,
    tape: &TapeDoc,
) -> Option<Vec<u32>> {
    let mut nodes = filter
        .kinds
        .iter()
        .filter_map(|kind| index.get(kind))
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    nodes.sort_unstable();
    let Some(needle) = &filter.needle else {
        return Some(nodes);
    };
    let source = tape.get_source().as_str();
    // Nested nodes' texts overlap: past the file's length, one search of the file per literal
    // reads less than one per node.
    let spanned = nodes
        .iter()
        .map(|&ordinal| tape.transducer_span(ordinal).len())
        .sum::<usize>();
    let mut starts = rustc_hash::FxHashMap::<&str, Vec<u32>>::default();
    let mut kept = Vec::new();
    for (index, &ordinal) in nodes.iter().enumerate() {
        if index.is_multiple_of(POLL_INTERVAL) && budget::matcher_budget_exhausted() {
            return None;
        }
        let holds = if spanned <= source.len() {
            needle.holds(tape.transducer_text_str(ordinal))
        } else {
            let span = tape.transducer_span(ordinal);
            needle.holds_with("", &mut |_, literal| {
                let starts = starts
                    .entry(literal)
                    .or_insert_with(|| occurrences(source, literal));
                let first = starts.partition_point(|&start| (start as usize) < span.start);
                starts
                    .get(first)
                    .is_some_and(|&start| start as usize + literal.len() <= span.end)
            })
        };
        if holds {
            kept.push(ordinal);
        }
    }
    Some(kept)
}

/// Where `literal` starts in `source`, overlapping occurrences included, in order.
fn occurrences(source: &str, literal: &str) -> Vec<u32> {
    let finder = memchr::memmem::Finder::new(literal.as_bytes());
    let bytes = source.as_bytes();
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(at) = bytes.get(from..).and_then(|rest| finder.find(rest)) {
        starts.push((from + at) as u32);
        from += at + 1;
    }
    starts
}

/// `file`'s nodes of `kinds`, in preorder, from the kind index of the file being evaluated (built
/// once for it); none where the deadline passed while the index was built (the poll that found it
/// recorded the overrun, which ends the file's evaluation).
pub(super) fn nodes_of_kinds(
    file: &Root<TapeDoc>,
    kinds: &std::collections::BTreeSet<u16>,
) -> Vec<u32> {
    let tape = file.root().get_doc();
    let collect = |index: &rustc_hash::FxHashMap<u16, Vec<u32>>| {
        let mut nodes = kinds
            .iter()
            .filter_map(|kind| index.get(kind))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        nodes.sort_unstable();
        nodes
    };
    let address = std::ptr::from_ref(file) as usize;
    MEMO.with_borrow_mut(|memo| {
        if memo.root != address {
            return kind_index(tape).as_ref().map_or_else(Vec::new, collect);
        }
        if memo.kinds.is_none() {
            memo.kinds = kind_index(tape);
        }
        memo.kinds.as_ref().map_or_else(Vec::new, collect)
    })
}

/// The nodes a two-valued walk over `range` (of `node`'s descendants, by ordinal) tests for a rule
/// `filter` bounds, in preorder: those of its kinds whose text holds its needle; none where the
/// deadline passed while they were listed (the check stops at it). `None` where the walk must test
/// every node: in a three-valued check (its order of unknown verdicts is the walk's), or over a
/// tree that is not the evaluated file's.
pub(super) fn bounded<'t, D: Doc>(
    filter: &Arc<WalkFilter>,
    node: &Node<'t, D>,
    range: impl FnOnce(&TapeDoc, u32) -> Range<u32>,
) -> Option<Vec<Node<'t, D>>> {
    let address = std::ptr::from_ref(node.root()) as usize;
    if TypeId::of::<D>() != TypeId::of::<TapeDoc>()
        || THREE_VALUED.get()
        || MEMO.with_borrow(|memo| memo.root != address)
    {
        return None;
    }
    // SAFETY: `D` is `TapeDoc` (checked above).
    let node: &Node<'t, TapeDoc> = unsafe { &*std::ptr::from_ref(node).cast() };
    let root = node.root();
    let tape = root.root().get_doc();
    let range = range(tape, node.node_id() as u32);
    let postings = MEMO.with_borrow_mut(|memo| {
        let key = Arc::as_ptr(filter) as usize;
        if let Some(postings) = memo.postings.get(&key) {
            return Some(Arc::clone(postings));
        }
        if memo.kinds.is_none() {
            memo.kinds = kind_index(tape);
        }
        let postings: Arc<[u32]> = postings(filter, memo.kinds.as_ref()?, tape)?.into();
        memo.postings.insert(key, Arc::clone(&postings));
        Some(postings)
    });
    let Some(postings) = postings else {
        return Some(Vec::new());
    };
    let start = postings.partition_point(|&ordinal| ordinal < range.start);
    let end = postings.partition_point(|&ordinal| ordinal < range.end);
    Some(
        postings[start..end]
            .iter()
            .map(|&ordinal| {
                let found = root.adopt(tape.node(ordinal));
                // SAFETY: `D` is `TapeDoc` (checked above).
                unsafe { same::<Node<'t, TapeDoc>, Node<'t, D>>(found) }
            })
            .collect(),
    )
}

/// A utility's verdict at `node` from `env`: remembered for the file being evaluated, else
/// `compute`d (which enters the utility).
pub(super) fn util<'t, D: Doc>(
    util: &UtilRef,
    node: Node<'t, D>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
    compute: impl FnOnce(Node<'t, D>, &mut Cow<MetaVarEnv<'t, D>>) -> Option<Node<'t, D>>,
) -> Option<Node<'t, D>> {
    let (Some(scope), Some(term)) = (util.scope(), util.memo_term()) else {
        return compute(node, env);
    };
    let keep = |name: &str| {
        scope
            .as_ref()
            .is_none_or(|scope| scope.binary_search_by(|kept| (**kept).cmp(name)).is_ok())
    };
    remember(term, &keep, node, env, compute)
}

/// A parameterized call's verdict at `node` from `env`: it reads the caller's bindings of the
/// captures it maps to its parameters alone (its frame holds nothing else).
pub(super) fn call<'t, D: Doc>(
    call: &UtilCall,
    node: Node<'t, D>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
    compute: impl FnOnce(Node<'t, D>, &mut Cow<MetaVarEnv<'t, D>>) -> Option<Node<'t, D>>,
) -> Option<Node<'t, D>> {
    let keep = |name: &str| call.args.iter().any(|(_, caller)| caller == name);
    remember(
        (std::ptr::from_ref(call) as usize, usize::MAX),
        &keep,
        node,
        env,
        compute,
    )
}

/// `compute`'s verdict at `node` for `term`, which reads the bindings `keep` names alone. Its
/// frame is live while the term is computed, through every level of utility recursion: the work
/// before and after runs in helpers of their own, so it stays small.
fn remember<'t, D: Doc>(
    term: (usize, usize),
    keep: &dyn Fn(&str) -> bool,
    node: Node<'t, D>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
    compute: impl FnOnce(Node<'t, D>, &mut Cow<MetaVarEnv<'t, D>>) -> Option<Node<'t, D>>,
) -> Option<Node<'t, D>> {
    let address = std::ptr::from_ref(node.root()) as usize;
    if TypeId::of::<D>() != TypeId::of::<TapeDoc>()
        || MEMO.with_borrow(|memo| memo.root != address)
        || UNKNOWN.get()
        || super::parse::hosting()
    {
        return compute(node, env);
    }
    // SAFETY: `D` is `TapeDoc` (checked above), so these are the same types.
    let (tape_node, tape_env) = unsafe {
        (
            &*std::ptr::from_ref(&node).cast::<Node<'t, TapeDoc>>(),
            &mut *std::ptr::from_mut(env).cast::<Cow<'_, MetaVarEnv<'t, TapeDoc>>>(),
        )
    };
    let key = match recall(term, keep, tape_node, tape_env) {
        Recalled::Verdict(matched) => return matched.then_some(node),
        Recalled::Missed(key) => key,
    };
    let started = Started::now();
    let matched = compute(node, env);
    // SAFETY: as above.
    let tape_env = unsafe { &*std::ptr::from_ref(env).cast::<Cow<'_, MetaVarEnv<'t, TapeDoc>>>() };
    remember_verdict(key, keep, matched.is_some(), tape_env, started);
    matched
}

enum Recalled {
    /// A remembered verdict, its bindings added to the environment.
    Verdict(bool),
    Missed(Box<Key>),
}

/// The remembered verdict for `term` at `node` from `env`, applied as computing it would have
/// been (its steps spent, its levels reached, its bindings added); else the key to remember it by.
#[inline(never)]
fn recall<'t>(
    term: (usize, usize),
    keep: &dyn Fn(&str) -> bool,
    node: &Node<'t, TapeDoc>,
    env: &mut Cow<MetaVarEnv<'t, TapeDoc>>,
) -> Recalled {
    let mut key = Box::new(Key {
        term,
        node: node.node_id() as u32,
        three: THREE_VALUED.get(),
        exclusion: each::exclusion_key(),
        following: super::binding::following_key(),
        input: bindings(env, &kept(keep)),
        owner: 0,
    });
    let found = |key: &Key| {
        MEMO.with_borrow(|memo| {
            memo.entry(key)
                .filter(|entry| util_levels_fit(entry.levels))
                .map(|entry| (entry.verdict.clone(), entry.steps, entry.levels))
        })
    };
    let mut hit = found(&key);
    if hit.is_none() {
        // Else this check's own: recalling it reads this check's state as computing it did.
        key.owner = super::resolve::CURRENT_CHECK.get();
        hit = found(&key);
        if hit.is_some() {
            read_check_state();
        }
    }
    let Some((verdict, steps, levels)) = hit else {
        return Recalled::Missed(key);
    };
    // What this verdict took counts toward the levels its callers' verdicts took.
    let reached = util_level() + levels;
    let peak = swap_util_peak(0);
    swap_util_peak(peak.max(reached));
    if !spend_util_steps(steps) {
        return Recalled::Verdict(false);
    }
    Recalled::Verdict(match verdict {
        Verdict::NoMatch => false,
        Verdict::Unknown => {
            UNKNOWN.set(true);
            false
        }
        Verdict::Match(added) => {
            if !added.is_empty() {
                let bound = env.to_mut();
                for (name, binding) in &added {
                    if bind(bound, name, binding, node).is_none() {
                        return Recalled::Verdict(false);
                    }
                }
            }
            true
        }
    })
}

/// `keep`, with an exported capture's source range (its reserved `\0` name) going with the capture
/// and, while `finding.each` excludes, the enumerated capture kept.
fn kept(keep: &dyn Fn(&str) -> bool) -> impl Fn(&str) -> bool + '_ {
    let excluding = each::active();
    move |name: &str| {
        let name = name.strip_prefix('\0').unwrap_or(name);
        keep(name) || excluding && each::excludes(name)
    }
}

/// What the budget and levels stood at when a computation started.
struct Started {
    steps: usize,
    fills: usize,
    reads: u64,
    checks: u64,
    levels: u32,
    outer: u32,
}

impl Started {
    fn now() -> Self {
        let levels = util_level();
        Self {
            steps: utility_steps_left(),
            fills: fill_steps(),
            reads: check_state_reads(),
            checks: checks_begun(),
            levels,
            outer: swap_util_peak(levels),
        }
    }
}

/// Remembers a computed verdict under `key`: what it bound of the names `keep` holds, the steps it
/// spent and the levels it went down, for the check in progress alone where it read that check's
/// state; nothing where it hit a limit or began a check of its own.
#[inline(never)]
fn remember_verdict(
    mut key: Box<Key>,
    keep: &dyn Fn(&str) -> bool,
    matched: bool,
    env: &MetaVarEnv<'_, TapeDoc>,
    started: Started,
) {
    let peak = swap_util_peak(0);
    swap_util_peak(started.outer.max(peak));
    if util_limit_hit() || checks_begun() != started.checks {
        return;
    }
    let verdict = if matched {
        Verdict::Match(
            bindings(env, &kept(keep))
                .into_iter()
                .filter(|binding| !key.input.contains(binding))
                .collect(),
        )
    } else if UNKNOWN.get() {
        Verdict::Unknown
    } else {
        Verdict::NoMatch
    };
    let entry = Entry {
        verdict,
        // The caches it filled are read for free when it is computed again.
        steps: started
            .steps
            .saturating_sub(utility_steps_left())
            .saturating_sub(fill_steps() - started.fills),
        levels: peak.saturating_sub(started.levels),
    };
    key.owner = if check_state_reads() == started.reads {
        0
    } else {
        super::resolve::CURRENT_CHECK.get()
    };
    MEMO.with_borrow_mut(|memo| memo.insert(*key, entry));
}

/// `nthChild`'s counted siblings under `parent` for the counting `term` (`0`: every named child,
/// which spends nothing; else its `nthChild`'s address, a check's own state), listed once per file
/// being evaluated, the steps that spent a cache fill (`filling`); elsewhere listed, and spent,
/// each time. `None` where `list` ran out of budget, which is not remembered.
pub(super) fn siblings<D: Doc>(
    term: usize,
    parent: &Node<'_, D>,
    list: impl FnOnce() -> Option<Siblings>,
) -> Option<Arc<Siblings>> {
    let address = std::ptr::from_ref(parent.root()) as usize;
    if MEMO.with_borrow(|memo| memo.root != address) {
        return list().map(Arc::new);
    }
    if term != 0 {
        read_check_state();
    }
    let key = (term, parent.node_id());
    if let Some(listed) = MEMO.with_borrow(|memo| memo.siblings.get(&key).cloned()) {
        return Some(listed);
    }
    let listed = Arc::new(filling(list)?);
    MEMO.with_borrow_mut(|memo| memo.siblings.insert(key, Arc::clone(&listed)));
    Some(listed)
}

/// `node` as a node of the file being evaluated, where it is one.
pub(super) fn file_node<'a, 't, D: Doc>(node: &'a Node<'t, D>) -> Option<&'a Node<'t, TapeDoc>> {
    let address = std::ptr::from_ref(node.root()) as usize;
    if TypeId::of::<D>() != TypeId::of::<TapeDoc>() || MEMO.with_borrow(|memo| memo.root != address)
    {
        return None;
    }
    // SAFETY: `D` is `TapeDoc` (checked above).
    Some(unsafe { &*std::ptr::from_ref(node).cast() })
}

/// A three-valued answer, ordered as `or` combines them.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Truth {
    False,
    Unknown,
    True,
}

/// A node-local relation's (`term`, its address) remembered answer at the evaluated file's `node`
/// (`FileMemo::answers`).
pub(super) fn answer(term: usize, node: usize, three: bool) -> Option<Truth> {
    MEMO.with_borrow(|memo| memo.answers.get(&(term, node, three)).copied())
}

/// Remembers a node-local relation's answers at the evaluated file's nodes (`answer`).
pub(super) fn remember_answers(term: usize, three: bool, answers: &[(usize, Truth)]) {
    if answers.is_empty() {
        return;
    }
    MEMO.with_borrow_mut(|memo| {
        memo.answers.extend(
            answers
                .iter()
                .map(|&(node, answer)| ((term, node, three), answer)),
        );
    });
}

/// One parent's children by their class under an `eq` comparison (`EqClass`), each list in
/// preorder, and whether classifying them spent steps.
struct SameKeys {
    children: Vec<u32>,
    texts: rustc_hash::FxHashMap<Vec<u16>, Vec<u32>>,
    unknown: Vec<u32>,
    spent: bool,
}

impl SameKeys {
    fn classify(comparison: &Comparison, parent: &Node<'_, TapeDoc>) -> Self {
        let steps = utility_steps_left();
        let mut keys = Self {
            children: Vec::new(),
            texts: rustc_hash::FxHashMap::default(),
            unknown: Vec::new(),
            spent: false,
        };
        for child in parent.children() {
            let ordinal = child.node_id() as u32;
            keys.children.push(ordinal);
            match comparison.eq_class(&child) {
                EqClass::Text(text) => keys.texts.entry(text).or_default().push(ordinal),
                EqClass::Unknown => keys.unknown.push(ordinal),
                EqClass::Never => {}
            }
        }
        keys.spent = utility_steps_left() != steps;
        keys
    }

    /// The children a node of `class` may compare equal to, or unknown with, in preorder.
    fn matching(&self, class: &EqClass) -> Cow<'_, [u32]> {
        match class {
            EqClass::Unknown => Cow::Borrowed(&self.children),
            EqClass::Never => Cow::Borrowed(&self.unknown),
            EqClass::Text(text) => match self.texts.get(text) {
                None => Cow::Borrowed(&self.unknown),
                Some(same) if self.unknown.is_empty() => Cow::Borrowed(same),
                Some(same) => {
                    let mut merged = [same.as_slice(), &self.unknown].concat();
                    merged.sort_unstable();
                    Cow::Owned(merged)
                }
            },
        }
    }
}

/// The siblings of the evaluated file's `node` past it (`forward`: after it, else before it, the
/// nearest first) that a node of `class` may compare equal to under `comparison`, or unknown with:
/// its parent's children are classified once per file (a cache fill, `filling`); a classification
/// the deadline or a limit cut short is not remembered. A classification that spent steps is the
/// check's own state: it is keyed by the check's `comparison`. `None` where `node` is not the
/// file's.
pub(super) fn same_key_siblings<'t, D: Doc>(
    comparison: &Comparison,
    node: &Node<'t, D>,
    class: &EqClass,
    forward: bool,
) -> Option<Vec<Node<'t, D>>> {
    let node = file_node(node)?;
    let Some(parent) = node.parent() else {
        return Some(Vec::new());
    };
    let key = (std::ptr::from_ref(comparison) as usize, parent.node_id());
    let keys = match MEMO.with_borrow(|memo| memo.same_keys.get(&key).cloned()) {
        Some(keys) => keys,
        None => {
            let keys = Arc::new(filling(|| SameKeys::classify(comparison, &parent)));
            if !util_limit_hit() && !budget::matcher_budget_observed() {
                MEMO.with_borrow_mut(|memo| memo.same_keys.insert(key, Arc::clone(&keys)));
            }
            keys
        }
    };
    if keys.spent {
        read_check_state();
    }
    let listed = keys.matching(class);
    let at = listed.partition_point(|&ordinal| (ordinal as usize) < node.node_id());
    let root = node.root();
    let tape = root.root().get_doc();
    let adopt = |&ordinal: &u32| {
        let found = root.adopt(tape.node(ordinal));
        // SAFETY: `D` is `TapeDoc` (`file_node` checked it).
        unsafe { same::<Node<'t, TapeDoc>, Node<'t, D>>(found) }
    };
    Some(if forward {
        listed[at..]
            .iter()
            .filter(|&&ordinal| ordinal as usize != node.node_id())
            .map(adopt)
            .collect()
    } else {
        listed[..at].iter().rev().map(adopt).collect()
    })
}

/// `env`'s bindings of the names `keep` holds.
fn bindings(env: &MetaVarEnv<'_, TapeDoc>, keep: &impl Fn(&str) -> bool) -> Bindings {
    let mut bindings = Vec::new();
    for variable in env.get_matched_variables() {
        match variable {
            MetaVariable::Capture(name, _) if keep(&name) => {
                if let Some(node) = env.get_match(&name) {
                    bindings.push((Arc::from(name), Bound::Single(node.node_id() as u32)));
                } else if let Some(bytes) = env.get_transformed(&name) {
                    bindings.push((Arc::from(name), Bound::Text(bytes.clone().into())));
                }
            }
            MetaVariable::MultiCapture(name) if keep(&name) => {
                let nodes = env
                    .get_multiple_matches(&name)
                    .iter()
                    .map(|node| node.node_id() as u32)
                    .collect();
                bindings.push((Arc::from(name), Bound::Multi(nodes)));
            }
            _ => {}
        }
    }
    bindings.sort_by(|(left, a): &(Arc<str>, Bound), (right, b)| {
        left.cmp(right).then_with(|| order(a).cmp(&order(b)))
    });
    bindings.dedup();
    bindings
}

fn order(bound: &Bound) -> u8 {
    match bound {
        Bound::Single(_) => 0,
        Bound::Multi(_) => 1,
        Bound::Text(_) => 2,
    }
}

/// Adds a remembered binding to `env`, its nodes from `node`'s document.
fn bind<'t>(
    env: &mut MetaVarEnv<'t, TapeDoc>,
    name: &str,
    bound: &Bound,
    node: &Node<'t, TapeDoc>,
) -> Option<()> {
    let root = node.root();
    let at = |ordinal: u32| root.adopt(root.root().get_doc().node(ordinal));
    match bound {
        Bound::Single(ordinal) => {
            env.insert(name, at(*ordinal))?;
        }
        Bound::Multi(ordinals) => {
            env.insert_multi(name, ordinals.iter().map(|&ordinal| at(ordinal)).collect())?;
        }
        Bound::Text(bytes) => {
            env.insert_transformation(&MetaVariable::Multiple, name, bytes.to_vec());
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::{WalkFilter, bounded, remembered_peak, reset_remembered_peak, with_file};
    use crate::policy_dsl::budget::MatchBudget;
    use crate::policy_dsl::language::GrammarDialect;
    use crate::policy_dsl::matcher::{FileScopeSlot, RequiredText};
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::test_support::*;
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn tape_file(source: &str, language: GrammarDialect) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(language).unwrap();
        let native = StrDoc::new(source, language);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        )
    }

    #[test]
    fn postings_the_deadline_cut_short_test_nothing_and_are_not_remembered() {
        let file = tape_file("f(g(h(1)), [k]);\n", GrammarDialect::TypeScript);
        let calls = file
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
            .expect("a call")
            .kind_id();
        let rule = |rule: Value| {
            let compiled =
                compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
            Arc::new(WalkFilter {
                kinds: [calls].into(),
                needle: RequiredText::of_node(&grammar_matchers(&compiled)[0].matcher),
            })
        };
        let (any, holding_g) = (
            rule(json!({ "kind": "call_expression" })),
            rule(json!({ "kind": "call_expression", "regex": "g" })),
        );
        let program = file.root();
        let tested = |filter: &Arc<WalkFilter>| {
            bounded(filter, &program, |tape, ordinal| {
                tape.transducer_descendants(ordinal)
            })
            .expect("a two-valued walk over the file")
            .len()
        };
        with_file(&file, || {
            // The kind index, then postings searched for a needle, each cut short.
            for (filter, listed) in [(&any, 3), (&holding_g, 2)] {
                let budget = MatchBudget::expired_for_test();
                assert_eq!(tested(filter), 0);
                assert!(budget.observed(), "the overrun was not recorded");
                drop(budget);
                assert_eq!(tested(filter), listed);
            }
        });
    }

    #[test]
    fn verdicts_of_past_enumeration_rounds_are_dropped() {
        const IDENTIFIERS: usize = 60;
        let source = format!(
            "f({});\n",
            (0..IDENTIFIERS)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut finding = finding();
        finding["each"] = json!("X");
        let compiled = compile_document_with(
            check_document(
                json!({ "id": "c", "violation": finding, "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "call_expression",
                          "has": { "kind": "arguments", "has": { "matches": "name" } } },
                "utils": { "name": { "kind": "identifier", "pattern": "$X" } },
            } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        reset_remembered_peak();
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        assert_findings(&result, IDENTIFIERS);
        // Round `n` remembers the `n` names it passes over again: every round's kept, the
        // verdicts would grow with the square of the names.
        assert!(
            remembered_peak() <= 3 * IDENTIFIERS,
            "{} verdicts remembered at once",
            remembered_peak()
        );
    }

    #[test]
    fn a_remembered_verdict_over_a_parsed_tree_replays_its_sibling_probes() {
        let file = tape_file("x: \"List[int, str]\"\n", GrammarDialect::Python);
        let compiled = compile_document_with(
            check_document(
                json!({ "id": "c", "violation": finding(), "forbid": { "ast": {
                "language": "python",
                "rule": { "matches": "annotated" },
                "utils": {
                    "annotated": { "kind": "string", "parse": { "grammar": "annotation",
                        "has": { "stopBy": "end", "matches": "second" } } },
                    "second": { "kind": "identifier",
                                "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } } },
                },
            } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let grammar = &grammar_matchers(&compiled)[0];
        let string = file
            .root()
            .dfs()
            .find(|node| node.kind() == "string")
            .expect("string");
        let spent = || {
            let mut work = 1_000;
            let matched = grammar.match_root(string.clone(), &mut work).expect("fits");
            assert!(matched.is_some());
            1_000 - work
        };
        let _slot = FileScopeSlot::install(Vec::new(), None, false);
        // The file's scope table, a cache of its own, is filled first.
        with_file(&file, spent);
        with_file(&file, || {
            reset_remembered_peak();
            let computed = spent();
            assert_eq!(remembered_peak(), 1, "the verdict was not remembered");
            // The annotation's siblings are not the file's: probing them fills no cache.
            assert_eq!(spent(), computed, "the remembered verdict spent less");
        });
    }

    #[test]
    fn a_verdict_reading_its_checks_own_caches_spends_for_another_check_what_computing_it_would() {
        let file = tape_file("f(a, b, c);\n", GrammarDialect::TypeScript);
        // Two checks with an equal context-free utility, whose verdicts are shared by digest; its
        // counted siblings are cached for the `nthChild` of one check alone.
        let compile = || {
            compile_document_with(
                check_document(
                    json!({ "id": "c", "violation": finding(), "forbid": { "ast": {
                    "language": "typescript",
                    "rule": { "matches": "second" },
                    "utils": { "second": { "kind": "identifier",
                        "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } } } },
                } } }),
                ),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason))
        };
        let (first, second) = (compile(), compile());
        let (first, second) = (&grammar_matchers(&first)[0], &grammar_matchers(&second)[0]);
        let b = file
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier" && node.text() == "b")
            .expect("b");
        let spent = |check: &crate::policy_dsl::matcher::GrammarMatcher| {
            let mut work = 1_000;
            let matched = check.match_root(b.clone(), &mut work).expect("fits");
            assert!(matched.is_some());
            1_000 - work
        };
        let alone = with_file(&file, || spent(second));
        let after_the_other = with_file(&file, || {
            spent(first);
            spent(second)
        });
        assert_eq!(after_the_other, alone, "another check's verdict spent less");
    }
}
