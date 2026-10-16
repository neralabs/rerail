//! The structural kernel: capture-free sub-rules (`kind`, `regex`, `value`, `all`, `any`, `not`,
//! relations that stop at the neighbour or walk to the end, and `matches` of a non-recursive
//! utility made only of these) answered natively at a node instead of by SQL walks over
//! `rerail_nodes` and utility statements.
//!
//! A sub-rule lowers to a graph of operations, shared across every clause of a catalog: equal
//! sub-rules become one operation. A document's answers are remembered per operation and node, so
//! a relation reuses its neighbours' answers instead of walking again: a node has a descendant
//! that matches when one of its children matches or has one; a node is inside a match when its
//! parent matches or is inside one; the sibling walks chain the same way. Node-local tests are
//! transducer programs (`Program::node_test`). Nothing is computed before a statement asks for it,
//! so a regex runs only at nodes a rule reaches.
//!
//! Such a sub-rule binds nothing and reads no capture: its answer at a node is the matcher's,
//! whatever order its terms run in. A `value` test may be unknown (in a three-valued clause; in a
//! two-valued one an unknown value is no match, as `tq_pred` reads it), and the answers combine
//! as the matcher's do when nothing binds: `all` is false when a term is, `any` and the relations
//! are true when a term or candidate is, and are otherwise unknown when one was; `not` keeps an
//! unknown. It spends no step of the utility work budget, which bounds recursion and fan-out: the
//! kernel answers each operation at most once per node of a file (a value decode spends its step,
//! once per node, as `tq_pred`'s does). Documents a sub-grammar parsed (`docs`) are answered by
//! the same walks the matcher makes.

use crate::code_rule_dsl::matcher::facts::test_predicate;
use crate::code_rule_dsl::matcher::{
    AstMatcher, Semantic, StopBy, UtilLimit, UtilitySummary, Verdict,
};
#[cfg(test)]
use crate::code_rule_dsl::test_support::{Walk, record_walk};
use crate::code_rule_dsl::transducer::sql_backend::BatchFile;
use crate::code_rule_dsl::transducer::{Error, Program};
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use regex::Regex;
use std::collections::HashMap;

#[cfg(test)]
thread_local! {
    /// Whether `value` tests stay `tq_pred` calls on this thread: the reference the kernel's
    /// answers are checked against.
    static VALUE_LOWERING_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Compiles `value` tests on this thread as predicate calls, not kernel operations, while `off`.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn set_value_lowering_off(off: bool) {
    VALUE_LOWERING_OFF.set(off);
}

fn value_lowers() -> bool {
    #[cfg(test)]
    if VALUE_LOWERING_OFF.get() {
        return false;
    }
    true
}

/// A three-valued answer, ordered so that `or` is the greater and `and` the lesser.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Truth {
    False,
    Unknown,
    True,
}

impl Truth {
    fn of(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
            Self::True => Self::False,
        }
    }
}

/// Why the kernel could not answer: a transducer error, or a limit a value decode hit.
#[derive(Debug)]
pub(super) enum Failure {
    Transducer(Error),
    Limit(UtilLimit),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Transducer(error)
    }
}

/// A `value` test's answer at `node`: unknown only in a three-valued clause (`three`).
fn value_truth<D: Doc>(
    test: &AstMatcher,
    three: bool,
    node: Node<'_, D>,
) -> Result<Truth, Failure> {
    match test_predicate(test, node, MetaVarEnv::new(), three, |_| None).map_err(Failure::Limit)? {
        Verdict::Match(_) => Ok(Truth::True),
        Verdict::NoMatch => Ok(Truth::False),
        Verdict::Unknown => Ok(Truth::Unknown),
    }
}

pub(super) type OpId = usize;

/// The node a relation reads its sub-rule at, as the matcher's relations walk.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Axis {
    /// `has`: some child.
    Child,
    /// `has` with `stopBy: end`: some descendant.
    Descendant,
    /// `inside`: the parent.
    Parent,
    /// `inside` with `stopBy: end`: some ancestor.
    Ancestor,
    /// `precedes`: the next sibling (ast-grep's `next`).
    Next,
    /// `precedes` with `stopBy: end`: some later sibling (ast-grep's `next_all`).
    After,
    /// `follows`: the previous sibling.
    Prev,
    /// `follows` with `stopBy: end`: some earlier sibling (ast-grep's `prev_all`).
    Before,
}

pub(super) enum Op {
    /// The node has every kind and its text matches every regex.
    Test {
        program: Program,
        kinds: Vec<u16>,
        regexes: Vec<Regex>,
    },
    /// A `value` test (`AstMatcher::Semantic(Semantic::Value)`), in a three-valued clause or not.
    Value {
        test: AstMatcher,
        three: bool,
    },
    All(Vec<OpId>),
    Any(Vec<OpId>),
    Not(OpId),
    Related(Axis, OpId),
}

#[derive(Eq, Hash, PartialEq)]
enum Key {
    Test(Vec<u16>, Vec<String>),
    All(Vec<OpId>),
    Any(Vec<OpId>),
    Not(OpId),
    Related(Axis, OpId),
}

/// The single kind id a kind rule matches.
fn kind_id(matcher: &ast_grep_core::matcher::KindMatcher) -> Option<u16> {
    let kinds = matcher.potential_kinds()?;
    let mut ids = kinds.iter();
    let id = ids.next()?;
    ids.next().is_none().then_some(id as u16)
}

/// Whether `matcher` is a sub-rule the kernel answers. A `matches` lowers when its utility's body
/// does and the utility does not reach itself: the kernel answers it without the utility's
/// statement, so it spends no step of the budget (its work is bounded by the file's nodes). A
/// `value` test reads only the node (and the file's bindings), never the environment.
pub(super) fn lowers(matcher: &AstMatcher) -> bool {
    lowers_within(matcher, &mut UtilitySummary::new())
}

fn lowers_within(
    matcher: &AstMatcher,
    utilities: &mut UtilitySummary<(usize, usize), bool>,
) -> bool {
    match matcher {
        AstMatcher::Kind(kind) => kind_id(kind).is_some(),
        AstMatcher::Regex(_) => true,
        AstMatcher::Semantic(Semantic::Value(_)) => value_lowers(),
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().all(|item| lowers_within(item, utilities))
        }
        AstMatcher::Not(item) => lowers_within(item, utilities),
        AstMatcher::Has(item, stop)
        | AstMatcher::Inside(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            matches!(stop, StopBy::Neighbor | StopBy::End) && lowers_within(item, utilities)
        }
        // A utility on a cycle reaches itself through a conjunction, so it never lowers.
        AstMatcher::Util(util) => utilities.summarize(
            (util.table.as_ptr() as usize, util.index),
            false,
            |utilities| {
                #[cfg(test)]
                record_walk(Walk::StructureLowers);
                util.with_target(|rule| lowers_within(rule, utilities))
                    .unwrap_or(false)
            },
        ),
        _ => false,
    }
}

/// The operations of a catalog, each distinct sub-rule once.
#[derive(Default)]
pub(super) struct Builder {
    ops: Vec<Op>,
    interned: HashMap<Key, OpId>,
    /// Each lowered utility's operation, in a two- or three-valued clause; a utility that lowers
    /// reaches no utility on a cycle.
    utilities: HashMap<(usize, usize, bool), OpId>,
}

impl Builder {
    /// The operation answering `matcher`, which `lowers`, in a three-valued clause (`three`) or not.
    /// A `value` test is not interned (it has no key to compare), so each lowering is its own
    /// operation.
    pub(super) fn lower(&mut self, matcher: &AstMatcher, three: bool) -> OpId {
        match matcher {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) => self.test(std::slice::from_ref(matcher)),
            AstMatcher::Semantic(Semantic::Value(_)) => {
                self.ops.push(Op::Value {
                    test: matcher.clone(),
                    three,
                });
                self.ops.len() - 1
            }
            AstMatcher::All(items) => {
                let (local, other): (Vec<_>, Vec<_>) = items
                    .iter()
                    .partition(|item| matches!(item, AstMatcher::Kind(_) | AstMatcher::Regex(_)));
                let mut ops = other
                    .into_iter()
                    .map(|item| self.lower(item, three))
                    .collect::<Vec<_>>();
                ops.sort_unstable();
                ops.dedup();
                if !local.is_empty() {
                    let local = local.into_iter().cloned().collect::<Vec<_>>();
                    // The node-local test first: it is the cheapest term.
                    ops.insert(0, self.test(&local));
                }
                if ops.len() == 1 {
                    return ops[0];
                }
                self.intern(Key::All(ops.clone()), || Op::All(ops))
            }
            AstMatcher::Any(items) => {
                let mut ops = items
                    .iter()
                    .map(|item| self.lower(item, three))
                    .collect::<Vec<_>>();
                ops.sort_unstable();
                ops.dedup();
                if ops.len() == 1 {
                    return ops[0];
                }
                self.intern(Key::Any(ops.clone()), || Op::Any(ops))
            }
            AstMatcher::Not(item) => {
                let op = self.lower(item, three);
                self.intern(Key::Not(op), || Op::Not(op))
            }
            AstMatcher::Has(item, stop) => {
                self.related(Axis::Child, Axis::Descendant, item, stop, three)
            }
            AstMatcher::Inside(item, stop) => {
                self.related(Axis::Parent, Axis::Ancestor, item, stop, three)
            }
            AstMatcher::Precedes(item, stop) => {
                self.related(Axis::Next, Axis::After, item, stop, three)
            }
            AstMatcher::Follows(item, stop) => {
                self.related(Axis::Prev, Axis::Before, item, stop, three)
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index, three);
                if let Some(&op) = self.utilities.get(&key) {
                    return op;
                }
                #[cfg(test)]
                record_walk(Walk::StructureLowering);
                let op = util
                    .with_target(|rule| self.lower(rule, three))
                    .expect("a filled utility");
                self.utilities.insert(key, op);
                op
            }
            _ => unreachable!("only a sub-rule that lowers"),
        }
    }

    fn related(
        &mut self,
        neighbour: Axis,
        end: Axis,
        item: &AstMatcher,
        stop: &StopBy,
        three: bool,
    ) -> OpId {
        let axis = match stop {
            StopBy::Neighbor => neighbour,
            StopBy::End => end,
            StopBy::Rule(_) => unreachable!("only a sub-rule that lowers"),
        };
        let op = self.lower(item, three);
        self.intern(Key::Related(axis, op), || Op::Related(axis, op))
    }

    /// One test of every kind and regex among `local`.
    fn test(&mut self, local: &[AstMatcher]) -> OpId {
        let mut kinds = Vec::new();
        let mut regexes = Vec::new();
        for item in local {
            match item {
                AstMatcher::Kind(kind) => kinds.push(kind_id(kind).expect("a single kind")),
                AstMatcher::Regex(regex) => regexes.push(regex.clone()),
                _ => unreachable!("node-local terms only"),
            }
        }
        kinds.sort_unstable();
        kinds.dedup();
        regexes.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        regexes.dedup_by(|left, right| left.as_str() == right.as_str());
        let key = Key::Test(
            kinds.clone(),
            regexes
                .iter()
                .map(|regex| regex.as_str().to_owned())
                .collect(),
        );
        self.intern(key, || Op::Test {
            program: Program::node_test(&kinds, &regexes).expect("a valid node test"),
            kinds,
            regexes,
        })
    }

    fn intern(&mut self, key: Key, make: impl FnOnce() -> Op) -> OpId {
        if let Some(&op) = self.interned.get(&key) {
            return op;
        }
        self.ops.push(make());
        let op = self.ops.len() - 1;
        self.interned.insert(key, op);
        op
    }

    pub(super) fn finish(self) -> Vec<Op> {
        self.ops
    }
}

/// Known answers, three bits per node: whether it is known, true, unknown.
struct Bits {
    known: Vec<u64>,
    value: Vec<u64>,
    unknown: Vec<u64>,
}

impl Bits {
    fn new(len: usize) -> Self {
        let words = len.div_ceil(64);
        Self {
            known: vec![0; words],
            value: vec![0; words],
            unknown: vec![0; words],
        }
    }

    fn get(&self, node: u32) -> Option<Truth> {
        let (word, bit) = (node as usize / 64, 1_u64 << (node % 64));
        (self.known[word] & bit != 0).then(|| {
            if self.value[word] & bit != 0 {
                Truth::True
            } else if self.unknown[word] & bit != 0 {
                Truth::Unknown
            } else {
                Truth::False
            }
        })
    }

    fn set(&mut self, node: u32, value: Truth) {
        let (word, bit) = (node as usize / 64, 1_u64 << (node % 64));
        self.known[word] |= bit;
        let assign = |words: &mut Vec<u64>, on: bool| {
            if on {
                words[word] |= bit;
            } else {
                words[word] &= !bit;
            }
        };
        assign(&mut self.value, value == Truth::True);
        assign(&mut self.unknown, value == Truth::Unknown);
    }
}

/// One file's remembered answers, by operation. For `After` and `Before` the bits hold the sibling
/// chain's answer (`Kernel::chain`), not the relation's.
#[derive(Default)]
pub(super) struct Memo {
    ops: Vec<Option<Bits>>,
}

impl Memo {
    fn bits(&mut self, op: OpId, len: usize) -> &mut Bits {
        if self.ops.len() <= op {
            self.ops.resize_with(op + 1, || None);
        }
        self.ops[op].get_or_insert_with(|| Bits::new(len))
    }

    fn get(&self, op: OpId, node: u32) -> Option<Truth> {
        self.ops.get(op)?.as_ref()?.get(node)
    }
}

/// `op`'s answer at `node` of a batch file, from and into `memo`.
pub(super) fn test_file(
    ops: &[Op],
    op: OpId,
    file: &BatchFile,
    memo: &mut Memo,
    node: u32,
) -> Result<Truth, Failure> {
    Kernel {
        ops,
        file,
        len: file.len(),
        memo,
    }
    .eval(op, node)
}

struct Kernel<'a> {
    ops: &'a [Op],
    file: &'a BatchFile,
    len: usize,
    memo: &'a mut Memo,
}

impl Kernel<'_> {
    fn set(&mut self, op: OpId, node: u32, value: Truth) {
        let len = self.len;
        self.memo.bits(op, len).set(node, value);
    }

    /// The `or` of `item` at `nodes`, ending at the first that is true.
    fn any(&mut self, item: OpId, nodes: impl Iterator<Item = u32>) -> Result<Truth, Failure> {
        let mut answer = Truth::False;
        for node in nodes {
            answer = answer.max(self.eval(item, node)?);
            if answer == Truth::True {
                break;
            }
        }
        Ok(answer)
    }

    fn eval(&mut self, op: OpId, node: u32) -> Result<Truth, Failure> {
        let ops = self.ops;
        let tape = self.file.tape();
        let chained = matches!(ops[op], Op::Related(Axis::After | Axis::Before, _));
        if !chained && let Some(known) = self.memo.get(op, node) {
            return Ok(known);
        }
        let value = match &ops[op] {
            Op::Test { program, .. } => Truth::of(program.accepts(tape, node)?),
            Op::Value { test, three } => value_truth(test, *three, self.file.node(node))?,
            Op::All(items) => {
                let mut all = Truth::True;
                for &item in items {
                    all = all.min(self.eval(item, node)?);
                    if all == Truth::False {
                        break;
                    }
                }
                all
            }
            Op::Any(items) => self.any_term(items, node)?,
            Op::Not(item) => self.eval(*item, node)?.not(),
            &Op::Related(axis, item) => match axis {
                Axis::Child => self.any(item, tape.transducer_children(node))?,
                Axis::Descendant => self.descendant(op, item, node)?,
                Axis::Parent => self.any(item, tape.transducer_parent(node).into_iter())?,
                Axis::Ancestor => {
                    self.chain(op, item, node, |tape, node| tape.transducer_parent(node))?
                }
                Axis::Next => {
                    let next = self
                        .file
                        .node(node)
                        .next()
                        .map(|next| next.node_id() as u32);
                    self.any(item, next.into_iter())?
                }
                Axis::Prev => self.any(item, tape.transducer_prev_sibling(node).into_iter())?,
                // ast-grep's sibling walks start at an anchor, then follow the sibling chain.
                Axis::After => match self.file.node(node).next_all().next() {
                    Some(first) => {
                        let first = first.node_id() as u32;
                        let own = self.eval(item, first)?;
                        if own == Truth::True {
                            return Ok(own);
                        }
                        return Ok(own.max(self.chain(op, item, first, |tape, node| {
                            tape.transducer_next_sibling(node, false)
                        })?));
                    }
                    None => return Ok(Truth::False),
                },
                Axis::Before => match self.file.node(node).prev_all().next() {
                    Some(first) => {
                        let first = first.node_id() as u32;
                        let own = self.eval(item, first)?;
                        if own == Truth::True {
                            return Ok(own);
                        }
                        return Ok(own.max(self.chain(op, item, first, |tape, node| {
                            tape.transducer_prev_sibling(node)
                        })?));
                    }
                    None => return Ok(Truth::False),
                },
            },
        };
        self.set(op, node, value);
        Ok(value)
    }

    /// The `or` of the `any`'s terms at `node`, ending at the first that is true.
    fn any_term(&mut self, items: &[OpId], node: u32) -> Result<Truth, Failure> {
        let mut any = Truth::False;
        for &item in items {
            any = any.max(self.eval(item, node)?);
            if any == Truth::True {
                break;
            }
        }
        Ok(any)
    }

    /// Whether `item` matches some node after `start` along `step`, remembered for every node the
    /// walk passed: each one's answer is the `or` of the item at the node after it and that node's
    /// own answer, so a match proves it for every node before it, and an unknown item for the
    /// nodes before that item only.
    fn chain(
        &mut self,
        op: OpId,
        item: OpId,
        start: u32,
        step: fn(&crate::code_rule_dsl::tape::TapeDoc, u32) -> Option<u32>,
    ) -> Result<Truth, Failure> {
        if let Some(known) = self.memo.get(op, start) {
            return Ok(known);
        }
        let tape = self.file.tape();
        let mut passed = vec![start];
        // The item's answer at the node after each passed node but the last.
        let mut verdicts = Vec::new();
        // The last passed node's answer.
        let mut answer = loop {
            let current = passed[passed.len() - 1];
            let Some(next) = step(tape, current) else {
                break Truth::False;
            };
            let verdict = self.eval(item, next)?;
            if verdict == Truth::True {
                break Truth::True;
            }
            if let Some(known) = self.memo.get(op, next) {
                break verdict.max(known);
            }
            verdicts.push(verdict);
            passed.push(next);
        };
        for (index, &node) in passed.iter().enumerate().rev() {
            self.set(op, node, answer);
            if index > 0 {
                answer = answer.max(verdicts[index - 1]);
            }
        }
        Ok(answer)
    }

    /// Whether `item` matches some descendant of `node`, walking its subtree in preorder and
    /// passing over subtrees whose answer is already known. A match proves it for every node from
    /// the match's parent up to `node`; a walk with neither a match nor an unknown answer
    /// disproves it for the whole subtree. One without a match but with an unknown answer is
    /// unknown for `node`; each node it walked then has the `or` of its children's answers and
    /// their items', computed bottom-up from the answers the walk remembered.
    fn descendant(&mut self, op: OpId, item: OpId, node: u32) -> Result<Truth, Failure> {
        let tape = self.file.tape();
        let range = tape.transducer_descendants(node);
        let mut candidate = range.start;
        let mut proven = None;
        let mut unknown = false;
        while candidate < range.end {
            let own = self.eval(item, candidate)?;
            if own == Truth::True {
                proven = tape.transducer_parent(candidate);
                break;
            }
            unknown |= own == Truth::Unknown;
            match self.memo.get(op, candidate) {
                Some(Truth::True) => {
                    proven = Some(candidate);
                    break;
                }
                Some(known) => {
                    unknown |= known == Truth::Unknown;
                    candidate = tape.transducer_descendants(candidate).end;
                }
                None => candidate += 1,
            }
        }
        if let Some(mut ancestor) = proven {
            while ancestor != node {
                self.set(op, ancestor, Truth::True);
                ancestor = tape
                    .transducer_parent(ancestor)
                    .expect("a descendant's ancestors reach it");
            }
            return Ok(Truth::True);
        }
        if !unknown {
            for descendant in range {
                self.set(op, descendant, Truth::False);
            }
            return Ok(Truth::False);
        }
        // The same walk again, over remembered answers only: each node walked, its item's answer
        // and whether its subtree was passed over.
        let mut walked = Vec::new();
        let mut below = vec![Truth::False; range.len()];
        let mut candidate = range.start;
        while candidate < range.end {
            let own = self.eval(item, candidate)?;
            let known = self.memo.get(op, candidate);
            walked.push((candidate, own, known.is_some()));
            match known {
                Some(known) => {
                    below[(candidate - range.start) as usize] = known;
                    candidate = tape.transducer_descendants(candidate).end;
                }
                None => candidate += 1,
            }
        }
        let mut answer = Truth::False;
        for &(candidate, own, _) in walked.iter().rev() {
            let value = own.max(below[(candidate - range.start) as usize]);
            match tape.transducer_parent(candidate) {
                Some(parent) if parent != node => {
                    let slot = &mut below[(parent - range.start) as usize];
                    *slot = (*slot).max(value);
                }
                _ => answer = answer.max(value),
            }
        }
        for (candidate, _, passed_over) in walked {
            if !passed_over {
                self.set(op, candidate, below[(candidate - range.start) as usize]);
            }
        }
        Ok(answer)
    }
}

/// `op`'s answer at `node` of a document a sub-grammar parsed, by the matcher's own walks.
pub(super) fn test_node<D: Doc>(
    ops: &[Op],
    op: OpId,
    node: &Node<'_, D>,
) -> Result<Truth, Failure> {
    fn any<'t, D: Doc>(
        ops: &[Op],
        item: OpId,
        nodes: impl Iterator<Item = Node<'t, D>>,
    ) -> Result<Truth, Failure> {
        let mut answer = Truth::False;
        for node in nodes {
            answer = answer.max(test_node(ops, item, &node)?);
            if answer == Truth::True {
                break;
            }
        }
        Ok(answer)
    }
    Ok(match &ops[op] {
        Op::Test { kinds, regexes, .. } => Truth::of(
            kinds.iter().all(|&kind| node.kind_id() == kind)
                && regexes.iter().all(|regex| regex.is_match(&node.text())),
        ),
        Op::Value { test, three } => value_truth(test, *three, node.clone())?,
        Op::All(items) => {
            let mut all = Truth::True;
            for &item in items {
                all = all.min(test_node(ops, item, node)?);
                if all == Truth::False {
                    break;
                }
            }
            all
        }
        Op::Any(items) => {
            let mut any = Truth::False;
            for &item in items {
                any = any.max(test_node(ops, item, node)?);
                if any == Truth::True {
                    break;
                }
            }
            any
        }
        Op::Not(item) => test_node(ops, *item, node)?.not(),
        &Op::Related(axis, item) => match axis {
            Axis::Child => any(ops, item, node.children())?,
            Axis::Descendant => any(ops, item, node.dfs().skip(1))?,
            Axis::Parent => any(ops, item, node.parent().into_iter())?,
            Axis::Ancestor => any(ops, item, node.ancestors())?,
            Axis::Next => any(ops, item, node.next().into_iter())?,
            Axis::After => any(ops, item, node.next_all())?,
            Axis::Prev => any(ops, item, node.prev().into_iter())?,
            Axis::Before => any(ops, item, node.prev_all())?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::test_support::*;

    /// Eligibility and lowering expand each utility of a diamond chain once, not once per path.
    #[test]
    fn utility_diamonds_lower_in_linear_visits() {
        let compiled = compiled_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        let bound = linear_walk(DIAMOND_DEPTH);
        reset_walks();
        assert!(lowers(matcher));
        assert!(
            walks(Walk::StructureLowers) <= bound,
            "eligibility expanded {} utilities (bound {bound})",
            walks(Walk::StructureLowers)
        );
        Builder::default().lower(matcher, false);
        assert!(
            walks(Walk::StructureLowering) <= bound,
            "lowering expanded {} utilities (bound {bound})",
            walks(Walk::StructureLowering)
        );
    }
}
