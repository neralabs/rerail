//! The node-set executor: a program's outputs as a set of starting nodes, computed one
//! instruction at a time over whole node sets instead of one path at a time.
//!
//! For a program without captures that matter (its outputs' captures and labels are dropped) and
//! whose control flow only goes forward (every `FORK`, `GOTO`, `TEST` and `COUNT` target lies
//! after the instruction), the nodes at which a path started at instruction `pc` emits are a
//! function of the sets of the instructions after it:
//!
//! - a test keeps the nodes that pass it and at which the next instruction's paths emit;
//! - a navigation keeps the nodes from which some reached node is in the next instruction's set;
//! - `FORK` is a union, `GOTO` its target's set, `TEST` an intersection (a difference when
//!   negated), `COUNT` the nodes whose count of reached nodes in its target's set is in range;
//! - `EMIT` keeps every node, `STOP` none, `CAPTURE` changes nothing.
//!
//! Each instruction's set is computed only on the nodes some path can be at there (its demand):
//! a forward pass propagates the starting nodes through the program, then a backward pass
//! computes each instruction's set on its demand from the sets after it. Both passes are linear
//! in the tree per instruction, whatever the number of starting nodes. This is the path
//! executor's semantics exactly (`Program::execute_roots`, whose outputs' starts are this set);
//! the tests check the two agree.

use super::{
    ANCESTOR, CAPTURE, CHILD, COUNT, DESCEND, EMIT, Error, FIELD, FIELD_FIRST, FLAGS, FORK, GOTO,
    KIND, NAMED, NEGATE, NEXT, PARENT, Program, SIZE, SIZE_LONG, STOP, TEST, TEXT, split,
};

/// The tree a program runs over, by preorder node id: a node's descendants are the ids after it
/// up to its `last`.
pub(in crate::code_rule_dsl) trait Tree {
    fn len(&self) -> u32;
    fn kind(&self, node: u32) -> u16;
    fn field(&self, node: u32) -> u16;
    /// Whether the node is its parent's first child with its (nonzero) field.
    fn field_first(&self, node: u32) -> bool;
    /// Bits `0=named`, `1=extra`, `2=missing`, `3=error`.
    fn flags(&self, node: u32) -> u8;
    fn parent(&self, node: u32) -> Option<u32>;
    /// The last node of the node's subtree (the node itself for a leaf).
    fn last(&self, node: u32) -> u32;
    fn first_child(&self, node: u32) -> Option<u32>;
    fn next_sibling(&self, node: u32) -> Option<u32>;
    fn text(&self, node: u32) -> &str;
    fn span_len(&self, node: u32) -> u64;
}

/// A set of node ids, one bit each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct NodeSet {
    words: Vec<u64>,
}

impl NodeSet {
    pub(in crate::code_rule_dsl) fn new(len: u32) -> Self {
        Self {
            words: vec![0; (len as usize).div_ceil(64)],
        }
    }

    /// Every node of a tree of `len` nodes.
    pub(in crate::code_rule_dsl) fn full(len: u32) -> Self {
        let mut set = Self {
            words: vec![!0; (len as usize).div_ceil(64)],
        };
        if !len.is_multiple_of(64)
            && let Some(last) = set.words.last_mut()
        {
            *last = (1 << (len % 64)) - 1;
        }
        set
    }

    pub(in crate::code_rule_dsl) fn insert(&mut self, node: u32) {
        self.words[node as usize / 64] |= 1 << (node % 64);
    }

    pub(in crate::code_rule_dsl) fn contains(&self, node: u32) -> bool {
        self.words
            .get(node as usize / 64)
            .is_some_and(|word| word & (1 << (node % 64)) != 0)
    }

    fn union_with(&mut self, other: &Self) {
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word |= other;
        }
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// The nodes in ascending order.
    pub(in crate::code_rule_dsl) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(index, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros();
                word &= word - 1;
                Some(index as u32 * 64 + bit)
            })
        })
    }

    /// Whether some node in `from..=to` is in the set.
    fn any_in(&self, from: u32, to: u32) -> bool {
        if from > to {
            return false;
        }
        let (first, last) = (from as usize / 64, to as usize / 64);
        let low = !0u64 << (from % 64);
        let high = !0u64 >> (63 - to % 64);
        if first == last {
            return self.words[first] & low & high != 0;
        }
        self.words[first] & low != 0
            || self.words[first + 1..last].iter().any(|word| *word != 0)
            || self.words[last] & high != 0
    }

    /// How many nodes in `from..=to` are in the set.
    fn count_in(&self, from: u32, to: u32) -> u64 {
        if from > to {
            return 0;
        }
        let (first, last) = (from as usize / 64, to as usize / 64);
        let low = !0u64 << (from % 64);
        let high = !0u64 >> (63 - to % 64);
        if first == last {
            return u64::from((self.words[first] & low & high).count_ones());
        }
        u64::from((self.words[first] & low).count_ones())
            + self.words[first + 1..last]
                .iter()
                .map(|word| u64::from(word.count_ones()))
                .sum::<u64>()
            + u64::from((self.words[last] & high).count_ones())
    }
}

fn selected(tree: &impl Tree, node: u32, mode: u8) -> bool {
    mode != NAMED || tree.flags(node) & 1 != 0
}

fn children(tree: &impl Tree, node: u32) -> impl Iterator<Item = u32> + '_ {
    std::iter::successors(tree.first_child(node), |&child| tree.next_sibling(child))
}

fn ancestors(tree: &impl Tree, node: u32) -> impl Iterator<Item = u32> + '_ {
    std::iter::successors(tree.parent(node), |&node| tree.parent(node))
}

/// The selected nodes of `set`: all of them, or its named ones.
fn restricted<'s>(tree: &impl Tree, set: &'s NodeSet, mode: u8) -> std::borrow::Cow<'s, NodeSet> {
    if mode != NAMED {
        return std::borrow::Cow::Borrowed(set);
    }
    let mut named = NodeSet::new(tree.len());
    for node in set.iter().filter(|&node| selected(tree, node, NAMED)) {
        named.insert(node);
    }
    std::borrow::Cow::Owned(named)
}

impl Program {
    /// Whether every control transfer goes forward, as the node-set executor requires.
    pub(in crate::code_rule_dsl) fn forward(&self) -> bool {
        self.words.iter().enumerate().all(|(pc, &word)| {
            let (opcode, _, operand) = split(word);
            match opcode {
                FORK | GOTO | TEST => usize::from(operand) > pc,
                COUNT => usize::from(self.count(operand).target) > pc,
                _ => true,
            }
        })
    }

    /// The nodes among `starts` at which a path started at instruction zero emits.
    pub(in crate::code_rule_dsl) fn holds(
        &self,
        tree: &impl Tree,
        starts: &NodeSet,
    ) -> Result<NodeSet, Error> {
        if !self.forward() {
            return Err(Error::UnsupportedPattern);
        }
        let len = tree.len();
        let size = self.words.len();
        let mut demand: Vec<NodeSet> = (0..size).map(|_| NodeSet::new(len)).collect();
        demand[0] = starts.clone();
        // The nodes a test passed, kept for the backward pass.
        let mut passed: Vec<Option<NodeSet>> = (0..size).map(|_| None).collect();
        for pc in 0..size {
            if demand[pc].is_empty() {
                continue;
            }
            let (opcode, mode, operand) = split(self.words[pc]);
            let here = std::mem::take(&mut demand[pc]);
            let mut next = NodeSet::new(len);
            match opcode {
                STOP | EMIT => {}
                KIND | FIELD | FLAGS | TEXT | SIZE => {
                    for node in here.iter() {
                        if self.passes(tree, node, opcode, mode, operand) {
                            next.insert(node);
                        }
                    }
                    passed[pc] = Some(next.clone());
                }
                CAPTURE | TEST => next.union_with(&here),
                COUNT => {
                    next.union_with(&here);
                    let count = self.count(operand);
                    let target = usize::from(count.target);
                    let mut reached = NodeSet::new(len);
                    self.reach(tree, &here, count.nav, count.mode, &mut reached);
                    demand[target].union_with(&reached);
                }
                CHILD | DESCEND | ANCESTOR | PARENT | NEXT => {
                    self.reach(tree, &here, opcode, mode, &mut next);
                }
                FORK => {
                    next.union_with(&here);
                    demand[usize::from(operand)].union_with(&here);
                }
                GOTO => demand[usize::from(operand)].union_with(&here),
                _ => unreachable!("validated opcode"),
            }
            if opcode == TEST {
                demand[usize::from(operand)].union_with(&here);
            }
            if !matches!(opcode, STOP | EMIT | GOTO) && !next.is_empty() {
                demand[pc + 1].union_with(&next);
            }
            demand[pc] = here;
        }
        // Backward: each instruction's set on its demand, from the sets after it.
        let mut sets: Vec<NodeSet> = (0..size).map(|_| NodeSet::new(0)).collect();
        for pc in (0..size).rev() {
            let here = &demand[pc];
            let mut set = NodeSet::new(len);
            if here.is_empty() {
                sets[pc] = set;
                continue;
            }
            let (opcode, mode, operand) = split(self.words[pc]);
            match opcode {
                STOP => {}
                EMIT => set = here.clone(),
                KIND | FIELD | FLAGS | TEXT | SIZE => {
                    let next = &sets[pc + 1];
                    for node in passed[pc].as_ref().expect("a test's passed nodes").iter() {
                        if next.contains(node) {
                            set.insert(node);
                        }
                    }
                }
                CAPTURE => {
                    let next = &sets[pc + 1];
                    for node in here.iter().filter(|&node| next.contains(node)) {
                        set.insert(node);
                    }
                }
                FORK => {
                    let (next, target) = (&sets[pc + 1], &sets[usize::from(operand)]);
                    for node in here.iter() {
                        if next.contains(node) || target.contains(node) {
                            set.insert(node);
                        }
                    }
                }
                GOTO => {
                    let target = &sets[usize::from(operand)];
                    for node in here.iter().filter(|&node| target.contains(node)) {
                        set.insert(node);
                    }
                }
                TEST => {
                    let (next, target) = (&sets[pc + 1], &sets[usize::from(operand)]);
                    let negate = mode == NEGATE;
                    for node in here.iter() {
                        if next.contains(node) && target.contains(node) != negate {
                            set.insert(node);
                        }
                    }
                }
                COUNT => {
                    let count = self.count(operand);
                    let next = &sets[pc + 1];
                    let target = restricted(tree, &sets[usize::from(count.target)], count.mode);
                    for node in here.iter().filter(|&node| next.contains(node)) {
                        let found = Self::count_reached(tree, node, count.nav, &target);
                        if found >= count.min && count.max.is_none_or(|max| found <= max) {
                            set.insert(node);
                        }
                    }
                }
                CHILD | DESCEND | ANCESTOR | PARENT | NEXT => {
                    let next = &sets[pc + 1];
                    let reached = restricted(tree, next, mode);
                    for node in here.iter() {
                        let holds = match opcode {
                            CHILD => children(tree, node).any(|child| reached.contains(child)),
                            DESCEND => reached.any_in(node + 1, tree.last(node)),
                            ANCESTOR => ancestors(tree, node).any(|up| reached.contains(up)),
                            PARENT => tree.parent(node).is_some_and(|up| next.contains(up)),
                            NEXT => Self::next_selected(tree, node, mode)
                                .is_some_and(|sibling| next.contains(sibling)),
                            _ => unreachable!("a navigation"),
                        };
                        if holds {
                            set.insert(node);
                        }
                    }
                }
                _ => unreachable!("validated opcode"),
            }
            sets[pc] = set;
        }
        Ok(std::mem::take(&mut sets[0]))
    }

    fn passes(&self, tree: &impl Tree, node: u32, opcode: u8, mode: u8, operand: u16) -> bool {
        match opcode {
            KIND => tree.kind(node) == operand,
            FIELD => tree.field(node) == operand && (mode != FIELD_FIRST || tree.field_first(node)),
            FLAGS => {
                let flags = tree.flags(node);
                flags & mode == mode && flags & operand as u8 == 0
            }
            TEXT => self.text_matches(operand, tree.text(node)),
            SIZE => {
                let actual = tree.span_len(node);
                let expected = if mode & SIZE_LONG == 0 {
                    u64::from(operand)
                } else {
                    self.longs[usize::from(operand)]
                };
                match mode & !SIZE_LONG {
                    0 => actual == expected,
                    1 => actual >= expected,
                    2 => actual <= expected,
                    _ => unreachable!("validated size mode"),
                }
            }
            _ => unreachable!("a test"),
        }
    }

    fn next_selected(tree: &impl Tree, node: u32, mode: u8) -> Option<u32> {
        std::iter::successors(tree.next_sibling(node), |&sibling| {
            tree.next_sibling(sibling)
        })
        .find(|&sibling| selected(tree, sibling, mode))
    }

    /// Adds to `out` every node navigation `nav` (mode `mode`) reaches from a node of `from`.
    fn reach(&self, tree: &impl Tree, from: &NodeSet, nav: u8, mode: u8, out: &mut NodeSet) {
        match nav {
            CHILD => {
                for node in from.iter() {
                    for child in children(tree, node).filter(|&c| selected(tree, c, mode)) {
                        out.insert(child);
                    }
                }
            }
            DESCEND => {
                // Subtrees nest in preorder: skip what an earlier node's subtree covered.
                let mut covered: Option<u32> = None;
                for node in from.iter() {
                    let last = tree.last(node);
                    let start = match covered {
                        Some(end) if end >= last => continue,
                        Some(end) if end > node => end + 1,
                        _ => node + 1,
                    };
                    for descendant in start..=last {
                        if selected(tree, descendant, mode) {
                            out.insert(descendant);
                        }
                    }
                    covered = Some(covered.map_or(last, |end| end.max(last)));
                }
            }
            PARENT => {
                for parent in from.iter().filter_map(|node| tree.parent(node)) {
                    out.insert(parent);
                }
            }
            ANCESTOR => {
                // Every ancestor once: stop climbing at one already added.
                let mut seen = NodeSet::new(tree.len());
                for node in from.iter() {
                    for up in ancestors(tree, node) {
                        if seen.contains(up) {
                            break;
                        }
                        seen.insert(up);
                        if selected(tree, up, mode) {
                            out.insert(up);
                        }
                    }
                }
            }
            NEXT => {
                for sibling in from
                    .iter()
                    .filter_map(|node| Self::next_selected(tree, node, mode))
                {
                    out.insert(sibling);
                }
            }
            _ => unreachable!("validated navigation"),
        }
    }

    /// How many nodes navigation `nav` reaches from `node` in `target` (already restricted to the
    /// navigation's selection).
    fn count_reached(tree: &impl Tree, node: u32, nav: u8, target: &NodeSet) -> u64 {
        match nav {
            CHILD => children(tree, node)
                .filter(|&child| target.contains(child))
                .count() as u64,
            DESCEND => target.count_in(node + 1, tree.last(node)),
            PARENT => u64::from(tree.parent(node).is_some_and(|up| target.contains(up))),
            ANCESTOR => ancestors(tree, node)
                .filter(|&up| target.contains(up))
                .count() as u64,
            _ => unreachable!("validated count navigation"),
        }
    }
}

/// A tape as the path executor reads it, for comparing the two executors.
#[cfg(test)]
impl Tree for crate::code_rule_dsl::tape::TapeDoc {
    fn len(&self) -> u32 {
        self.transducer_len() as u32
    }

    fn kind(&self, node: u32) -> u16 {
        self.transducer_kind(node)
    }

    fn field(&self, node: u32) -> u16 {
        self.transducer_field(node)
    }

    fn field_first(&self, node: u32) -> bool {
        let field = self.transducer_field(node);
        field != 0
            && self.transducer_parent(node).is_some_and(|parent| {
                self.transducer_children(parent)
                    .find(|&child| self.transducer_field(child) == field)
                    == Some(node)
            })
    }

    fn flags(&self, node: u32) -> u8 {
        self.transducer_flags(node)
    }

    fn parent(&self, node: u32) -> Option<u32> {
        self.transducer_parent(node)
    }

    fn last(&self, node: u32) -> u32 {
        self.transducer_descendants(node).end - 1
    }

    fn first_child(&self, node: u32) -> Option<u32> {
        self.transducer_children(node).next()
    }

    fn next_sibling(&self, node: u32) -> Option<u32> {
        self.transducer_next_sibling(node, false)
    }

    fn text(&self, node: u32) -> &str {
        self.transducer_text_str(node)
    }

    fn span_len(&self, node: u32) -> u64 {
        self.transducer_span(node).len() as u64
    }
}
