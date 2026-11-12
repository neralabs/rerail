//! `nthChild` (ast-grep-config 0.45.3 `rule/nth_child.rs`): the node's 1-based position among its
//! parent's named children, or among those `ofRule` matches, counted from the end with `reverse`.

use super::AstMatcher;
use super::CaptureAnalysis;
use super::spend_util_step;
use super::util::{THREE_VALUED, UNKNOWN};
use ast_grep_core::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;

/// CSS's `An+B`: a position `index` (1-based) matches where `index = step * n + offset` for some
/// `n >= 0`; a zero step matches `offset` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub step: i32,
    pub offset: i32,
}

impl Position {
    /// Upstream's `FunctionalPosition::is_matched`, integer division and remainder included.
    pub fn holds(self, index: usize) -> bool {
        let Some(index) = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
        else {
            return false;
        };
        let step = i64::from(self.step);
        let offset = i64::from(self.offset);
        if step == 0 {
            index == offset
        } else {
            let Some(n) = index.checked_sub(offset) else {
                return false;
            };
            n / step >= 0 && n % step == 0
        }
    }
}

#[derive(Clone)]
pub struct NthChild {
    pub position: Position,
    pub of_rule: Option<AstMatcher>,
    /// `of_rule`'s identity: equal keys count the same siblings (`Positions`); `None` where it is
    /// not shared (it calls a utility).
    pub(crate) of_rule_key: Option<String>,
    pub reverse: bool,
    /// Whether `of_rule` reads and binds no capture (`CaptureAnalysis::mentions`), worked out on
    /// first use: its verdict at a sibling is then the same for every candidate.
    pub(crate) capture_free: std::sync::OnceLock<bool>,
}

impl NthChild {
    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let index = self.index(&node, env)?;
        self.position.holds(index).then_some(node)
    }

    /// The node's 0-based index among the counted siblings. Each sibling is probed against a copy
    /// of the bindings, so one sibling's bindings never reject the next; the node's own bindings
    /// under `ofRule` are then committed. A sibling whose verdict is unknown leaves the index unknown.
    /// Where `ofRule` reads no capture, a parent's siblings are probed once per file (`memo`).
    fn index<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<usize> {
        let parent = node.parent()?;
        let named = || {
            parent.children().filter(|child| {
                let named = child.is_named();
                #[cfg(any(test, feature = "test-hooks"))]
                if named {
                    NAMED_SIBLING_VISITS.set(NAMED_SIBLING_VISITS.get() + 1);
                }
                named
            })
        };
        let siblings = match &self.of_rule {
            None => super::memo::siblings(0, &parent, || {
                Some(Siblings {
                    counted: named().map(|child| child.node_id()).collect(),
                    unknown: false,
                })
            })?,
            Some(rule) => {
                let three = THREE_VALUED.get();
                let saved = three && UNKNOWN.replace(false);
                let free = *self
                    .capture_free
                    .get_or_init(|| CaptureAnalysis::default().mentions(rule).is_empty());
                let list = || {
                    let mut siblings = Siblings {
                        counted: Vec::new(),
                        unknown: false,
                    };
                    for child in named() {
                        if !spend_util_step() {
                            return None;
                        }
                        let mut probe = Cow::Borrowed(env.as_ref());
                        let id = child.node_id();
                        let matched = super::each::suspended(|| {
                            rule.match_node_with_env(child, &mut probe).is_some()
                        });
                        if matched {
                            siblings.counted.push(id);
                        } else if three && UNKNOWN.replace(false) {
                            siblings.unknown = true;
                            break;
                        }
                    }
                    Some(siblings)
                };
                let listed = if free {
                    super::memo::siblings(std::ptr::from_ref(self) as usize, &parent, list)
                } else {
                    list().map(std::sync::Arc::new)
                };
                // The budget ran out: no verdict.
                let Some(listed) = listed else {
                    if three {
                        UNKNOWN.set(saved || UNKNOWN.get());
                    }
                    return None;
                };
                if listed.unknown {
                    UNKNOWN.set(true);
                    return None;
                }
                if three {
                    UNKNOWN.set(saved);
                }
                listed
            }
        };
        let position = siblings.counted.binary_search(&node.node_id()).ok()?;
        let index = if self.reverse {
            siblings.counted.len() - 1 - position
        } else {
            position
        };
        if let Some(rule) = &self.of_rule {
            rule.match_node_with_env(node.clone(), env);
        }
        Some(index)
    }
}

impl NthChild {
    /// The nodes of `starts` (ascending) at which the predicate holds on `file`'s tape, as
    /// `match_node` decides them (a fact's rows, decision 0030), in one pass per parent: each parent
    /// of a start lists its counted children once, and each start's index is a search in that list.
    /// `ofRule` is read off the tape's kinds where it is only `kind`, `all`, `any` and `not`, else
    /// tested by W's matcher once per child. A child whose verdict is unknown leaves its parent's
    /// children without one (no rows), as `index` does.
    pub(in crate::code_rule_dsl) fn rows(
        &self,
        file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        starts: impl Iterator<Item = u32>,
    ) -> Vec<u32> {
        let tape = file.root().get_doc();
        let mut counted = std::collections::HashMap::<u32, Option<Vec<u32>>>::new();
        let mut rows = Vec::new();
        for node in starts {
            let Some(parent) = tape.transducer_parent(node) else { continue };
            let list = counted.entry(parent).or_insert_with(|| self.counted(file, tape, parent));
            let Some(list) = list else { continue };
            let Ok(position) = list.binary_search(&node) else { continue };
            let index = if self.reverse { list.len() - 1 - position } else { position };
            if self.position.holds(index) {
                rows.push(node);
            }
        }
        rows
    }

    /// `rows` as bits over the tape's nodes, for starts of `kinds` (any node where `None`), from
    /// the counted children of each parent (`Positions`): the child at the position where `An+B`
    /// names one (`B` alone), else each counted child tested. No pass over the tape's nodes.
    pub(in crate::code_rule_dsl) fn bits(
        &self,
        tape: &crate::code_rule_dsl::tape::TapeDoc,
        positions: &Positions,
        kinds: Option<&std::collections::BTreeSet<u16>>,
    ) -> Vec<u64> {
        let mut bits = vec![0_u64; tape.transducer_len().div_ceil(64)];
        let mut mark = |node: u32| {
            if kinds.is_none_or(|kinds| kinds.contains(&tape.transducer_kind(node))) {
                bits[node as usize / 64] |= 1 << (node % 64);
            }
        };
        for &parent in &positions.parents {
            let counted = positions.counted(parent);
            let at = |index: usize| if self.reverse { counted[counted.len() - 1 - index] } else { counted[index] };
            if self.position.step == 0 {
                // `holds(index)` is `index + 1 == B`.
                if let Ok(index) = usize::try_from(i64::from(self.position.offset) - 1)
                    && index < counted.len()
                {
                    mark(at(index));
                }
            } else {
                for index in 0..counted.len() {
                    if self.position.holds(index) {
                        mark(at(index));
                    }
                }
            }
        }
        bits
    }

    /// W's matcher's verdict of `rule` at `child`; `None` where it is unknown.
    fn probe(&self, file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>, rule: &AstMatcher, child: u32) -> Option<bool> {
        use super::util::{UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STEPS_LEFT};
        THREE_VALUED.set(true);
        UTIL_DEPTH.set(0);
        UTIL_EXCEEDED.set(None);
        UTIL_STEPS_LEFT.set(usize::MAX);
        UNKNOWN.set(false);
        let mut env = Cow::Owned(MetaVarEnv::new());
        let matched = rule.match_node_with_env(super::node_at(file, child), &mut env).is_some();
        (!UNKNOWN.replace(false)).then_some(matched)
    }

    fn counted(
        &self,
        file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        tape: &crate::code_rule_dsl::tape::TapeDoc,
        parent: u32,
    ) -> Option<Vec<u32>> {
        let mut list = Vec::new();
        self.count_into(file, tape, parent, &mut list).then_some(list)
    }

    /// `parent`'s counted children into `list`; `false` where a child's verdict is unknown.
    fn count_into(
        &self,
        file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        tape: &crate::code_rule_dsl::tape::TapeDoc,
        parent: u32,
        list: &mut Vec<u32>,
    ) -> bool {
        list.clear();
        for child in tape.transducer_children(parent) {
            if !tape.transducer_is_named(child) {
                continue;
            }
            let counts = match &self.of_rule {
                None => true,
                Some(rule) => match on_kinds(rule, tape.transducer_kind(child)) {
                    Some(verdict) => verdict,
                    None => {
                        use super::util::{UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STEPS_LEFT};
                        THREE_VALUED.set(true);
                        UTIL_DEPTH.set(0);
                        UTIL_EXCEEDED.set(None);
                        UTIL_STEPS_LEFT.set(usize::MAX);
                        UNKNOWN.set(false);
                        let mut env = Cow::Owned(MetaVarEnv::new());
                        let node = super::node_at(file, child);
                        let matched = rule.match_node_with_env(node, &mut env).is_some();
                        if UNKNOWN.replace(false) {
                            return false;
                        }
                        matched
                    }
                },
            };
            if counts {
                list.push(child);
            }
        }
        true
    }
}

/// `rule`'s verdict at a node of kind `kind`, where the rule tests only kinds.
/// A tape's counted children by parent: each parent's named children (those its `ofRule`
/// matches, where it has one), in order. A parent with a child of unknown verdict counts none of
/// them (no rows, as `rows` has it).
pub(in crate::code_rule_dsl) struct Positions {
    /// The parents with a counted child, ascending.
    parents: Vec<u32>,
    /// `counted[offsets[parent]..offsets[parent + 1]]` is the parent's.
    offsets: Vec<u32>,
    counted: Vec<u32>,
}

impl Positions {
    fn counted(&self, parent: u32) -> &[u32] {
        &self.counted[self.offsets[parent as usize] as usize..self.offsets[parent as usize + 1] as usize]
    }

    /// One pass over each parent's children, appending the counted ones; a parent with a child of
    /// unknown verdict takes its run back.
    fn of(nth: &NthChild, tape: &crate::code_rule_dsl::tape::TapeDoc) -> Self {
        let len = tape.transducer_len();
        // W's matcher reads the file through its own handle and scope slot, made where a probe is.
        let file = std::cell::OnceCell::new();
        let file = || {
            file.get_or_init(|| {
                let scopes = super::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty());
                (ast_grep_core::AstGrep::doc(tape.clone()), scopes)
            })
        };
        // Per kind: 0 not yet asked, 1 no, 2 yes, 3 W's matcher decides.
        let mut by_kind = Vec::<u8>::new();
        let mut counts = |node: u32| -> Option<bool> {
            if !tape.transducer_is_named(node) {
                return Some(false);
            }
            let Some(rule) = &nth.of_rule else { return Some(true) };
            let kind = usize::from(tape.transducer_kind(node));
            if by_kind.len() <= kind {
                by_kind.resize(kind + 1, 0);
            }
            if by_kind[kind] == 0 {
                by_kind[kind] = match on_kinds(rule, kind as u16) {
                    Some(false) => 1,
                    Some(true) => 2,
                    None => 3,
                };
            }
            match by_kind[kind] {
                1 => Some(false),
                2 => Some(true),
                _ => nth.probe(&file().0, rule, node),
            }
        };
        let mut parents = Vec::new();
        let mut offsets = vec![0_u32; len + 1];
        let mut counted = Vec::with_capacity(len);
        for parent in 0..len as u32 {
            let from = counted.len();
            for child in tape.transducer_children(parent) {
                match counts(child) {
                    Some(true) => counted.push(child),
                    Some(false) => {}
                    None => {
                        counted.truncate(from);
                        break;
                    }
                }
            }
            if counted.len() > from {
                parents.push(parent);
            }
            offsets[parent as usize + 1] = counted.len() as u32;
        }
        Self { parents, offsets, counted }
    }
}

/// A tape's `Positions` by `ofRule` (none, or its key), and the answers of its `nthChild` facts by
/// question (`Fact::question`): each made on first use and shared by the tape's facts.
#[derive(Default)]
pub struct SiblingPositions {
    positions: std::sync::Mutex<std::collections::HashMap<Option<String>, std::sync::Arc<Positions>>>,
    answers: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Vec<u64>>>>,
}

impl SiblingPositions {
    pub(in crate::code_rule_dsl) fn of(&self, nth: &NthChild, tape: &crate::code_rule_dsl::tape::TapeDoc) -> std::sync::Arc<Positions> {
        // A rule without a key is its own.
        if nth.of_rule.is_some() && nth.of_rule_key.is_none() {
            return std::sync::Arc::new(Positions::of(nth, tape));
        }
        let mut positions = self.positions.lock().expect("sibling positions");
        std::sync::Arc::clone(positions.entry(nth.of_rule_key.clone()).or_insert_with(|| std::sync::Arc::new(Positions::of(nth, tape))))
    }

    /// The answer to `question`, made by `answer` the first time it is asked.
    pub(in crate::code_rule_dsl) fn answer(&self, question: &str, answer: impl FnOnce() -> Vec<u64>) -> std::sync::Arc<Vec<u64>> {
        if let Some(found) = self.answers.lock().expect("nthChild answers").get(question) {
            return std::sync::Arc::clone(found);
        }
        let found = std::sync::Arc::new(answer());
        self.answers.lock().expect("nthChild answers").insert(question.to_owned(), std::sync::Arc::clone(&found));
        found
    }
}

fn on_kinds(rule: &AstMatcher, kind: u16) -> Option<bool> {
    match rule {
        AstMatcher::Kind(matcher) => Some(matcher.potential_kinds()?.contains(usize::from(kind))),
        AstMatcher::All(items) => items.iter().try_fold(true, |all, item| Some(all && on_kinds(item, kind)?)),
        AstMatcher::Any(items) => items.iter().try_fold(false, |any, item| Some(any || on_kinds(item, kind)?)),
        AstMatcher::Not(item) => Some(!on_kinds(item, kind)?),
        _ => None,
    }
}

/// A parent's named children counted by `nthChild` (all, or those its `ofRule` matches), by node
/// id in order; `unknown` where a probe's verdict was.
pub(super) struct Siblings {
    pub(super) counted: Vec<usize>,
    pub(super) unknown: bool,
}

/// Upstream's `parse_an_b`: white space is ignored; `n`/`N` with an optional signed step, then an
/// optional signed offset; or a bare signed number.
pub fn parse_position(input: &str) -> Result<Position, String> {
    enum State {
        Initial,
        N,
        Sign(bool),
        Num(bool),
    }
    let syntax = || "invalid syntax".to_owned();
    let (mut step, mut sign, mut num) = (0i32, 1i32, 0i32);
    let mut state = State::Initial;
    let digit = |c: char| c as i32 - '0' as i32;
    for c in input.chars().filter(|c| !c.is_whitespace()) {
        state = match (state, c) {
            (State::Initial, '+' | '-') => {
                sign = if c == '+' { 1 } else { -1 };
                State::Sign(false)
            }
            (State::Initial, '0'..='9') => {
                num = digit(c);
                State::Num(false)
            }
            (State::Initial, 'n' | 'N') => {
                step = sign;
                State::N
            }
            (State::Sign(_), '+' | '-') => return Err(syntax()),
            (State::Sign(has_n), '0'..='9') => {
                num = digit(c);
                State::Num(has_n)
            }
            (State::Sign(true), 'n' | 'N') => return Err(syntax()),
            (State::Sign(false), 'n' | 'N') => {
                step = sign;
                State::N
            }
            (State::Num(_), '+' | '-') => return Err(syntax()),
            (State::Num(has_n), '0'..='9') => {
                num = num
                    .checked_mul(10)
                    .and_then(|num| num.checked_add(digit(c)))
                    .ok_or_else(syntax)?;
                State::Num(has_n)
            }
            (State::Num(true), 'n' | 'N') => return Err(syntax()),
            (State::Num(false), 'n' | 'N') => {
                step = sign * num;
                num = 0;
                State::N
            }
            (State::N, '+' | '-') => {
                sign = if c == '+' { 1 } else { -1 };
                num = 0;
                State::Sign(true)
            }
            (State::N, '0'..='9' | 'n' | 'N') => return Err(syntax()),
            (_, other) => return Err(format!("illegal character `{other}`")),
        };
    }
    match state {
        State::Initial | State::Sign(_) => Err(syntax()),
        State::N | State::Num(_) => Ok(Position {
            step,
            offset: num * sign,
        }),
    }
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static NAMED_SIBLING_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Named children `nthChild` listed on this thread (`reset_named_sibling_visits` restarts it).
#[cfg(any(test, feature = "test-hooks"))]
pub fn named_sibling_visits() -> usize {
    NAMED_SIBLING_VISITS.get()
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn reset_named_sibling_visits() {
    NAMED_SIBLING_VISITS.set(0);
}

// cratify: `tests` is rerail-engine's (it mounts it)

