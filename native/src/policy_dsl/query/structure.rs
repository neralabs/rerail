//! The structural kernel: capture-free, two-valued sub-rules (`kind`, `regex`, `all`, `any`,
//! `not`, relations that stop at the neighbour or walk to the end, and `matches` of a
//! non-recursive utility made only of these) answered natively at a node instead of by SQL walks
//! over `rerail_nodes` and utility statements.
//!
//! A sub-rule lowers to a graph of operations, shared across every check of a catalog: equal
//! sub-rules become one operation. A document's answers are remembered per operation and node, so
//! a relation reuses its neighbours' answers instead of walking again: a node has a descendant
//! that matches when one of its children matches or has one; a node is inside a match when its
//! parent matches or is inside one; the sibling walks chain the same way. Node-local tests are
//! transducer programs (`Program::node_test`). Nothing is computed before a statement asks for it,
//! so a regex runs only at nodes a rule reaches.
//!
//! Such a sub-rule binds nothing, reads no capture and has no unknown verdict: its answer at a
//! node is the matcher's, whatever order its terms run in. It spends no step of the utility work
//! budget, which bounds recursion and fan-out: the kernel answers each operation at most once per
//! node of a file. Documents a sub-grammar parsed (`docs`) are answered by the same walks the
//! matcher makes.

use crate::policy_dsl::matcher::{AstMatcher, StopBy, UtilitySummary};
#[cfg(test)]
use crate::policy_dsl::test_support::{Walk, record_walk};
use crate::policy_dsl::transducer::sql_backend::BatchFile;
use crate::policy_dsl::transducer::{Error, Program};
use ast_grep_core::matcher::Matcher;
use ast_grep_core::{Doc, Node};
use regex::Regex;
use std::collections::HashMap;

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
/// statement, so it spends no step of the budget (its work is bounded by the file's nodes).
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
    /// Each lowered utility's operation; a utility that lowers reaches no utility on a cycle.
    utilities: HashMap<(usize, usize), OpId>,
}

impl Builder {
    /// The operation answering `matcher`, which `lowers`.
    pub(super) fn lower(&mut self, matcher: &AstMatcher) -> OpId {
        match matcher {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) => self.test(std::slice::from_ref(matcher)),
            AstMatcher::All(items) => {
                let (local, other): (Vec<_>, Vec<_>) = items
                    .iter()
                    .partition(|item| matches!(item, AstMatcher::Kind(_) | AstMatcher::Regex(_)));
                let mut ops = other
                    .into_iter()
                    .map(|item| self.lower(item))
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
                    .map(|item| self.lower(item))
                    .collect::<Vec<_>>();
                ops.sort_unstable();
                ops.dedup();
                if ops.len() == 1 {
                    return ops[0];
                }
                self.intern(Key::Any(ops.clone()), || Op::Any(ops))
            }
            AstMatcher::Not(item) => {
                let op = self.lower(item);
                self.intern(Key::Not(op), || Op::Not(op))
            }
            AstMatcher::Has(item, stop) => self.related(Axis::Child, Axis::Descendant, item, stop),
            AstMatcher::Inside(item, stop) => {
                self.related(Axis::Parent, Axis::Ancestor, item, stop)
            }
            AstMatcher::Precedes(item, stop) => self.related(Axis::Next, Axis::After, item, stop),
            AstMatcher::Follows(item, stop) => self.related(Axis::Prev, Axis::Before, item, stop),
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                if let Some(&op) = self.utilities.get(&key) {
                    return op;
                }
                #[cfg(test)]
                record_walk(Walk::StructureLowering);
                let op = util
                    .with_target(|rule| self.lower(rule))
                    .expect("a filled utility");
                self.utilities.insert(key, op);
                op
            }
            _ => unreachable!("only a sub-rule that lowers"),
        }
    }

    fn related(&mut self, neighbour: Axis, end: Axis, item: &AstMatcher, stop: &StopBy) -> OpId {
        let axis = match stop {
            StopBy::Neighbor => neighbour,
            StopBy::End => end,
            StopBy::Rule(_) => unreachable!("only a sub-rule that lowers"),
        };
        let op = self.lower(item);
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

/// Known answers, one bit pair per node.
struct Bits {
    known: Vec<u64>,
    value: Vec<u64>,
}

impl Bits {
    fn new(len: usize) -> Self {
        let words = len.div_ceil(64);
        Self {
            known: vec![0; words],
            value: vec![0; words],
        }
    }

    fn get(&self, node: u32) -> Option<bool> {
        let (word, bit) = (node as usize / 64, 1_u64 << (node % 64));
        (self.known[word] & bit != 0).then(|| self.value[word] & bit != 0)
    }

    fn set(&mut self, node: u32, value: bool) {
        let (word, bit) = (node as usize / 64, 1_u64 << (node % 64));
        self.known[word] |= bit;
        if value {
            self.value[word] |= bit;
        } else {
            self.value[word] &= !bit;
        }
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

    fn get(&self, op: OpId, node: u32) -> Option<bool> {
        self.ops.get(op)?.as_ref()?.get(node)
    }
}

/// Whether `op` matches at `node` of a batch file, answering from and into `memo`.
pub(super) fn test_file(
    ops: &[Op],
    op: OpId,
    file: &BatchFile,
    memo: &mut Memo,
    node: u32,
) -> Result<bool, Error> {
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
    fn set(&mut self, op: OpId, node: u32, value: bool) {
        let len = self.len;
        self.memo.bits(op, len).set(node, value);
    }

    fn eval(&mut self, op: OpId, node: u32) -> Result<bool, Error> {
        let ops = self.ops;
        let tape = self.file.tape();
        let chained = matches!(ops[op], Op::Related(Axis::After | Axis::Before, _));
        if !chained && let Some(known) = self.memo.get(op, node) {
            return Ok(known);
        }
        let value = match &ops[op] {
            Op::Test { program, .. } => program.accepts(tape, node)?,
            Op::All(items) => {
                let mut all = true;
                for &item in items {
                    if !self.eval(item, node)? {
                        all = false;
                        break;
                    }
                }
                all
            }
            Op::Any(items) => {
                let mut any = false;
                for &item in items {
                    if self.eval(item, node)? {
                        any = true;
                        break;
                    }
                }
                any
            }
            Op::Not(item) => !self.eval(*item, node)?,
            &Op::Related(axis, item) => match axis {
                Axis::Child => {
                    let mut any = false;
                    for child in tape.transducer_children(node) {
                        if self.eval(item, child)? {
                            any = true;
                            break;
                        }
                    }
                    any
                }
                Axis::Descendant => self.descendant(op, item, node)?,
                Axis::Parent => match tape.transducer_parent(node) {
                    Some(parent) => self.eval(item, parent)?,
                    None => false,
                },
                Axis::Ancestor => {
                    self.chain(op, item, node, |tape, node| tape.transducer_parent(node))?
                }
                Axis::Next => match self.file.node(node).next() {
                    Some(next) => self.eval(item, next.node_id() as u32)?,
                    None => false,
                },
                Axis::Prev => match tape.transducer_prev_sibling(node) {
                    Some(prev) => self.eval(item, prev)?,
                    None => false,
                },
                // ast-grep's sibling walks start at an anchor, then follow the sibling chain.
                Axis::After => match self.file.node(node).next_all().next() {
                    Some(first) => {
                        let first = first.node_id() as u32;
                        return Ok(self.eval(item, first)?
                            || self.chain(op, item, first, |tape, node| {
                                tape.transducer_next_sibling(node, false)
                            })?);
                    }
                    None => return Ok(false),
                },
                Axis::Before => match self.file.node(node).prev_all().next() {
                    Some(first) => {
                        let first = first.node_id() as u32;
                        return Ok(self.eval(item, first)?
                            || self.chain(op, item, first, |tape, node| {
                                tape.transducer_prev_sibling(node)
                            })?);
                    }
                    None => return Ok(false),
                },
            },
        };
        self.set(op, node, value);
        Ok(value)
    }

    /// Whether `item` matches some node after `start` along `step`, remembered for every node the
    /// walk passed: each of them has the same answer.
    fn chain(
        &mut self,
        op: OpId,
        item: OpId,
        start: u32,
        step: fn(&crate::policy_dsl::tape::TapeDoc, u32) -> Option<u32>,
    ) -> Result<bool, Error> {
        if let Some(known) = self.memo.get(op, start) {
            return Ok(known);
        }
        let tape = self.file.tape();
        let mut passed = vec![start];
        let mut current = start;
        let found = loop {
            let Some(next) = step(tape, current) else {
                break false;
            };
            if self.eval(item, next)? {
                break true;
            }
            if let Some(known) = self.memo.get(op, next) {
                break known;
            }
            passed.push(next);
            current = next;
        };
        for node in passed {
            self.set(op, node, found);
        }
        Ok(found)
    }

    /// Whether `item` matches some descendant of `node`, walking its subtree in preorder and
    /// passing over subtrees already known to hold no match. A match proves it for every node from
    /// the match's parent up to `node`; no match disproves it for the whole subtree.
    fn descendant(&mut self, op: OpId, item: OpId, node: u32) -> Result<bool, Error> {
        let tape = self.file.tape();
        let range = tape.transducer_descendants(node);
        let mut candidate = range.start;
        let mut proven = None;
        while candidate < range.end {
            if self.eval(item, candidate)? {
                proven = tape.transducer_parent(candidate);
                break;
            }
            match self.memo.get(op, candidate) {
                Some(true) => {
                    proven = Some(candidate);
                    break;
                }
                Some(false) => candidate = tape.transducer_descendants(candidate).end,
                None => candidate += 1,
            }
        }
        match proven {
            Some(mut ancestor) => {
                while ancestor != node {
                    self.set(op, ancestor, true);
                    ancestor = tape
                        .transducer_parent(ancestor)
                        .expect("a descendant's ancestors reach it");
                }
                Ok(true)
            }
            None => {
                for descendant in range {
                    self.set(op, descendant, false);
                }
                Ok(false)
            }
        }
    }
}

/// Whether `op` matches at `node` of a document a sub-grammar parsed, by the matcher's own walks.
pub(super) fn test_node<D: Doc>(ops: &[Op], op: OpId, node: &Node<'_, D>) -> bool {
    let test = |item: OpId, node: &Node<'_, D>| test_node(ops, item, node);
    match &ops[op] {
        Op::Test { kinds, regexes, .. } => {
            kinds.iter().all(|&kind| node.kind_id() == kind)
                && regexes.iter().all(|regex| regex.is_match(&node.text()))
        }
        Op::All(items) => items.iter().all(|&item| test(item, node)),
        Op::Any(items) => items.iter().any(|&item| test(item, node)),
        Op::Not(item) => !test(*item, node),
        &Op::Related(axis, item) => match axis {
            Axis::Child => node.children().any(|child| test(item, &child)),
            Axis::Descendant => node.dfs().skip(1).any(|child| test(item, &child)),
            Axis::Parent => node.parent().is_some_and(|parent| test(item, &parent)),
            Axis::Ancestor => node.ancestors().any(|ancestor| test(item, &ancestor)),
            Axis::Next => node.next().is_some_and(|next| test(item, &next)),
            Axis::After => node.next_all().any(|next| test(item, &next)),
            Axis::Prev => node.prev().is_some_and(|prev| test(item, &prev)),
            Axis::Before => node.prev_all().any(|prev| test(item, &prev)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::test_support::*;

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
        Builder::default().lower(matcher);
        assert!(
            walks(Walk::StructureLowering) <= bound,
            "lowering expanded {} utilities (bound {bound})",
            walks(Walk::StructureLowering)
        );
    }
}
