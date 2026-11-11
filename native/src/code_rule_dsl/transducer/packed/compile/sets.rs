//! Label programs as node-set formulas (decision 0036: tree axes are tape built-ins, not walks).
//!
//! Most label passes ask a one-step question of every node: "a node of kind K whose child in field
//! F has label L", "a node with a descendant of label L", "a node with labels A and B". Run as a
//! walk from every start, that is an interpreted program per node, and a descendant step re-reads
//! the subtree from each ancestor (nodes × depth). As a formula over node sets it is a few
//! bitwise operations and one pass over the members of the inner set:
//!
//! - a node test (`KIND`, `FIELD`, `FLAGS`, `SUPERTYPE` — a grammar class or a label —, `TEXT`)
//!   is a set read from the tape's index, the labels or the text rows;
//! - consecutive tests are an intersection, a `FORK … GOTO` pair a union;
//! - an axis step (`CHILD`, `DESCEND`, `PARENT`) followed by tests at the node it reaches is
//!   `∃ axis. ψ`: the parents of ψ's members, all their ancestors (stopping at one already
//!   marked), or their children. Work follows ψ's members, not the tape's nodes;
//! - `SAVE … RESTORE` scopes a step: the tests after it read the start again;
//! - a following-sibling step (`FOLLOWING`) is `∃ later sibling. ψ`, one pass over each parent's
//!   children;
//! - a chain of child steps (`CHILD; FORK x; ψ; GOTO back; x: φ`, a relation's `stopBy`) is a least
//!   fixpoint, `T = φ ∨ (ψ ∧ ∃ child. T)`, one pass in reverse preorder (a child's id is above its
//!   parent's).
//!
//! A targets label asks the other way: not the starts whose walk emits but the nodes the walk
//! captures. Where the capture is the walk's last act, those are the image of the starts under the
//! walk's steps (`Set::Reached`): the children, descendants, parents or later siblings of a set's
//! members, or a chain's reach, each one pass forward.
//!
//! A program outside this fragment (a next-sibling step, a repetition, a capture other than the
//! last, a branch that moves and does not come back) is not a formula, and runs as a walk.

use super::*;
use crate::code_rule_dsl::tape::{DERIVED_CLASS_BASE, TapeDoc};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A set of nodes, as a formula over a tape's columns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Set {
    All,
    Kind(u16),
    Field(u16),
    /// `flags & mode == mode && flags & arg == 0` (`FLAGS`).
    Flags { mode: u8, arg: u8 },
    /// A grammar supertype, or a label (from `DERIVED_CLASS_BASE`).
    Class(u16),
    /// A text slot's row.
    Text(u16),
    And(Vec<Set>),
    Or(Vec<Set>),
    /// Nodes with a child (named, where `named`) in the set.
    Child { named: bool, of: Box<Set> },
    /// Nodes with a proper descendant (named, where `named`) in the set.
    Descendant { named: bool, of: Box<Set> },
    /// Nodes whose parent is in the set.
    Parent(Box<Set>),
    /// Nodes with a following sibling (named, where `named`) in the set.
    Following { named: bool, of: Box<Set> },
    /// Nodes with a chain of children (named, where `named`) `c1 … ck`, `c1 … c(k-1)` in `through`
    /// and `ck` in `to`.
    Chain { named: bool, through: Box<Set>, to: Box<Set> },
    /// The nodes a step reaches from the members of `from` (named, where `named`).
    Reached { step: Step, named: bool, from: Box<Set> },
}

/// A step from a node, as a targets label's image follows it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Step {
    Child,
    Descendant,
    Parent,
    Following,
    /// Child steps for as long as the node reached is in the set: every node on the way.
    Chain(Box<Set>),
}

impl Set {
    /// The starts from which `words` reaches its `EMIT` (a roots label's verdicts), where the
    /// program is a formula.
    pub(in crate::code_rule_dsl) fn of(words: &[u32]) -> Option<Self> {
        let mut end = words.len();
        if end == 0 || split(words[end - 1]).0 != EMIT {
            return None;
        }
        end -= 1;
        if end > 0 && split(words[end - 1]).0 == CAPTURE {
            end -= 1;
        }
        parse(words, 0, end).map(|(set, _)| set)
    }

    /// The nodes `words` captures on a path that emits from a member of `starts` (a targets
    /// label's), where the capture is the walk's last act and the program is a formula.
    pub(in crate::code_rule_dsl) fn targets(words: &[u32], starts: Self) -> Option<Self> {
        let end = words.len().checked_sub(2)?;
        if split(words[end + 1]).0 != EMIT || split(words[end]).0 != CAPTURE || words[..end].iter().any(|&word| split(word).0 == CAPTURE) {
            return None;
        }
        forward(words, 0, end, starts)
    }

    fn and(parts: Vec<Self>) -> Self {
        let mut flat = Vec::new();
        for part in parts {
            match part {
                Self::All => {}
                Self::And(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Self::All,
            1 => flat.pop().expect("one part"),
            _ => Self::And(flat),
        }
    }
}

/// The formula of `words[from..to]` at the node it starts from, and whether the code leaves the
/// node moved (an axis step not restored).
fn parse(words: &[u32], from: usize, to: usize) -> Option<(Set, bool)> {
    let mut parts = Vec::new();
    let mut pc = from;
    while pc < to {
        let (op, mode, arg) = split(words[pc]);
        match op {
            KIND => parts.push(Set::Kind(arg)),
            FIELD => parts.push(Set::Field(arg)),
            FLAGS => parts.push(Set::Flags { mode, arg: u8::try_from(arg).ok()? }),
            SUPERTYPE => parts.push(Set::Class(arg)),
            TEXT => parts.push(Set::Text(arg)),
            SAVE_CURSOR => {
                let restore = (pc + 1..to).find(|&at| matches!(split(words[at]), (RESTORE_CURSOR, _, cursor) if cursor == arg))?;
                parts.push(parse(words, pc + 1, restore)?.0);
                pc = restore + 1;
                continue;
            }
            // A chain of child steps: the rest of the region read where it stops.
            CHILD if let Some((through, after)) = chain(words, pc, to) => {
                let (through, _) = parse(words, pc + 2, through).filter(|(_, moved)| !moved)?;
                let to = Box::new(parse(words, after, to)?.0);
                parts.push(Set::Chain { named: mode == NAMED, through: Box::new(through), to });
                return Some((Set::and(parts), true));
            }
            // The rest of the region is read at the node the step reaches.
            CHILD | DESCEND | PARENT | FOLLOWING => {
                let named = mode == NAMED;
                let of = Box::new(parse(words, pc + 1, to)?.0);
                parts.push(match op {
                    CHILD => Set::Child { named, of },
                    DESCEND => Set::Descendant { named, of },
                    FOLLOWING => Set::Following { named, of },
                    _ => Set::Parent(of),
                });
                return Some((Set::and(parts), true));
            }
            // `FORK a; first…; GOTO b; a: second…; b:` — either branch, then the rest; each branch
            // must leave the node where it found it.
            FORK => {
                let at = usize::from(arg);
                if at <= pc + 1 || at > to {
                    return None;
                }
                let (goto, _, join) = split(words[at - 1]);
                let join = usize::from(join);
                if goto != GOTO || join < at || join > to {
                    return None;
                }
                let (first, moved_first) = parse(words, pc + 1, at - 1)?;
                let (second, moved_second) = parse(words, at, join)?;
                if moved_first || moved_second {
                    return None;
                }
                parts.push(Set::Or(vec![first, second]));
                pc = join;
                continue;
            }
            _ => return None,
        }
        pc += 1;
    }
    Some((Set::and(parts), false))
}

/// `CHILD` at `pc` as a loop: `pc: CHILD; FORK a; ψ…; GOTO pc; a:`. Where it is one, `ψ`'s end
/// (the `GOTO`) and `a`, where the walk goes on from the node the chain stops at.
fn chain(words: &[u32], pc: usize, to: usize) -> Option<(usize, usize)> {
    let (fork, _, at) = split(*words.get(pc + 1)?);
    let at = usize::from(at);
    if fork != FORK || at <= pc + 2 || at > to {
        return None;
    }
    let (goto, _, back) = split(words[at - 1]);
    (goto == GOTO && usize::from(back) == pc).then_some((at - 1, at))
}

/// The nodes `words[from..to]` reaches from the members of `starts`, its tests applied where they
/// are read.
fn forward(words: &[u32], from: usize, to: usize, starts: Set) -> Option<Set> {
    let mut at = starts;
    let mut pc = from;
    let test = |at: Set, test: Set| Set::and(vec![at, test]);
    while pc < to {
        let (op, mode, arg) = split(words[pc]);
        let named = mode == NAMED;
        match op {
            KIND => at = test(at, Set::Kind(arg)),
            FIELD => at = test(at, Set::Field(arg)),
            FLAGS => at = test(at, Set::Flags { mode, arg: u8::try_from(arg).ok()? }),
            SUPERTYPE => at = test(at, Set::Class(arg)),
            TEXT => at = test(at, Set::Text(arg)),
            SAVE_CURSOR => {
                let restore = (pc + 1..to).find(|&at| matches!(split(words[at]), (RESTORE_CURSOR, _, cursor) if cursor == arg))?;
                at = test(at, parse(words, pc + 1, restore)?.0);
                pc = restore + 1;
                continue;
            }
            CHILD if let Some((through, after)) = chain(words, pc, to) => {
                let (through, _) = parse(words, pc + 2, through).filter(|(_, moved)| !moved)?;
                at = Set::Reached { step: Step::Chain(Box::new(through)), named, from: Box::new(at) };
                pc = after;
                continue;
            }
            CHILD | DESCEND | PARENT | FOLLOWING => {
                let step = match op {
                    CHILD => Step::Child,
                    DESCEND => Step::Descendant,
                    FOLLOWING => Step::Following,
                    _ => Step::Parent,
                };
                at = Set::Reached { step, named, from: Box::new(at) };
            }
            // Either branch from here: the union of where each goes.
            FORK => {
                let fork = usize::from(arg);
                if fork <= pc + 1 || fork > to {
                    return None;
                }
                let (goto, _, join) = split(words[fork - 1]);
                let join = usize::from(join);
                if goto != GOTO || join < fork || join > to {
                    return None;
                }
                let first = forward(words, pc + 1, fork - 1, at.clone())?;
                let second = forward(words, fork, join, at)?;
                at = Set::Or(vec![first, second]);
                pc = join;
                continue;
            }
            _ => return None,
        }
        pc += 1;
    }
    Some(at)
}

/// A tape's columns as sets for one labelling, each built the first time a formula reads it.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Index {
    kinds: OnceLock<Postings>,
    fields: OnceLock<Postings>,
    flags: OnceLock<Vec<u8>>,
    /// Grammar supertypes asked for, by class.
    classes: Mutex<HashMap<u16, Arc<Vec<u64>>>>,
    /// Sibling positions and `nthChild` answers, for facts that are node sets.
    pub(in crate::code_rule_dsl) siblings: crate::code_rule_dsl::matcher::SiblingPositions,
}

/// A column's nodes by value, in order: `nodes[offsets[value]..offsets[value + 1]]`. Built by a
/// counting sort, two passes over the column.
struct Postings {
    offsets: Vec<u32>,
    nodes: Vec<u32>,
}

impl Postings {
    fn of(len: u32, value: impl Fn(u32) -> u16) -> Self {
        let mut offsets = Vec::<u32>::new();
        for node in 0..len {
            let value = usize::from(value(node));
            if offsets.len() < value + 2 {
                offsets.resize(value + 2, 0);
            }
            offsets[value + 1] += 1;
        }
        for at in 1..offsets.len() {
            offsets[at] += offsets[at - 1];
        }
        let mut next = offsets.clone();
        let mut nodes = vec![0_u32; len as usize];
        for node in 0..len {
            let value = usize::from(value(node));
            nodes[next[value] as usize] = node;
            next[value] += 1;
        }
        Self { offsets, nodes }
    }

    fn get(&self, value: u16) -> &[u32] {
        let value = usize::from(value);
        match (self.offsets.get(value), self.offsets.get(value + 1)) {
            (Some(&from), Some(&to)) => &self.nodes[from as usize..to as usize],
            _ => &[],
        }
    }
}

/// What a formula reads on one tape.
pub(in crate::code_rule_dsl) struct Reader<'a> {
    pub(in crate::code_rule_dsl) tape: &'a TapeDoc,
    pub(in crate::code_rule_dsl) index: &'a Index,
    /// The labels so far, by class less `DERIVED_CLASS_BASE`.
    pub(in crate::code_rule_dsl) labels: &'a [Vec<u64>],
    /// The program's text rows, by slot.
    pub(in crate::code_rule_dsl) texts: &'a [Vec<u64>],
}

impl Reader<'_> {
    fn words(&self) -> usize {
        self.tape.transducer_len().div_ceil(64)
    }

    fn postings(&self, nodes: &[u32]) -> Vec<u64> {
        let mut bits = vec![0_u64; self.words()];
        for &node in nodes {
            bits[node as usize / 64] |= 1 << (node % 64);
        }
        bits
    }

    fn flags(&self) -> &[u8] {
        self.index.flags.get_or_init(|| (0..self.tape.transducer_len() as u32).map(|node| self.tape.transducer_flags(node)).collect())
    }

    /// The members of `set`.
    pub(in crate::code_rule_dsl) fn eval(&self, set: &Set) -> Vec<u64> {
        let words = self.words();
        let len = self.tape.transducer_len();
        match set {
            Set::All => {
                let mut bits = vec![u64::MAX; words];
                if len % 64 != 0
                    && let Some(last) = bits.last_mut()
                {
                    *last = (1 << (len % 64)) - 1;
                }
                bits
            }
            Set::Kind(kind) => {
                let len = len as u32;
                self.postings(self.index.kinds.get_or_init(|| Postings::of(len, |node| self.tape.transducer_kind(node))).get(*kind))
            }
            Set::Field(field) => {
                let len = len as u32;
                self.postings(self.index.fields.get_or_init(|| Postings::of(len, |node| self.tape.transducer_field(node))).get(*field))
            }
            Set::Flags { mode, arg } => {
                let mut bits = vec![0_u64; words];
                for (node, &flags) in self.flags().iter().enumerate() {
                    if flags & mode == *mode && flags & arg == 0 {
                        bits[node / 64] |= 1 << (node % 64);
                    }
                }
                bits
            }
            Set::Class(class) if *class >= DERIVED_CLASS_BASE => {
                let mut bits = self.labels.get(usize::from(class - DERIVED_CLASS_BASE)).cloned().unwrap_or_default();
                bits.resize(words, 0);
                bits
            }
            Set::Class(class) => {
                let mut classes = self.index.classes.lock().expect("class sets");
                let bits = classes.entry(*class).or_insert_with(|| {
                    let mut bits = vec![0_u64; words];
                    for node in 0..len as u32 {
                        if self.tape.transducer_has_supertype(node, *class) {
                            bits[node as usize / 64] |= 1 << (node % 64);
                        }
                    }
                    Arc::new(bits)
                });
                bits.to_vec()
            }
            Set::Text(slot) => {
                let mut bits = self.texts.get(usize::from(*slot)).cloned().unwrap_or_default();
                bits.resize(words, 0);
                bits
            }
            Set::And(parts) => {
                // The cheapest first would help; the parts are few.
                let mut bits = self.eval(&parts[0]);
                for part in &parts[1..] {
                    if bits.iter().all(|&word| word == 0) {
                        break;
                    }
                    for (word, other) in bits.iter_mut().zip(self.eval(part)) {
                        *word &= other;
                    }
                }
                bits
            }
            Set::Or(parts) => {
                let mut bits = vec![0_u64; words];
                for part in parts {
                    for (word, other) in bits.iter_mut().zip(self.eval(part)) {
                        *word |= other;
                    }
                }
                bits
            }
            Set::Child { named, of } => {
                let of = self.eval(of);
                let mut bits = vec![0_u64; words];
                for node in ones(&of) {
                    if *named && self.flags()[node as usize] & NAMED == 0 {
                        continue;
                    }
                    if let Some(parent) = self.tape.transducer_parent(node) {
                        bits[parent as usize / 64] |= 1 << (parent % 64);
                    }
                }
                bits
            }
            Set::Descendant { named, of } => {
                let of = self.eval(of);
                let mut bits = vec![0_u64; words];
                for node in ones(&of) {
                    if *named && self.flags()[node as usize] & NAMED == 0 {
                        continue;
                    }
                    // Up to the first ancestor already marked: its own ancestors are.
                    let mut at = self.tape.transducer_parent(node);
                    while let Some(parent) = at {
                        let (word, bit) = (parent as usize / 64, 1 << (parent % 64));
                        if bits[word] & bit != 0 {
                            break;
                        }
                        bits[word] |= bit;
                        at = self.tape.transducer_parent(parent);
                    }
                }
                bits
            }
            Set::Parent(of) => {
                let of = self.eval(of);
                let mut bits = vec![0_u64; words];
                for node in ones(&of) {
                    for child in self.tape.transducer_children(node) {
                        bits[child as usize / 64] |= 1 << (child % 64);
                    }
                }
                bits
            }
            Set::Following { named, of } => {
                // Each member's parent's children, last first: a child is in once a later one is.
                let of = self.eval(of);
                let mut bits = vec![0_u64; words];
                let mut parents = vec![0_u64; words];
                let mut children = Vec::new();
                for node in ones(&of) {
                    let Some(parent) = self.tape.transducer_parent(node) else { continue };
                    if get(&parents, parent) {
                        continue;
                    }
                    put(&mut parents, parent);
                    children.clear();
                    children.extend(self.tape.transducer_children(parent));
                    let mut later = false;
                    for &child in children.iter().rev() {
                        if later {
                            put(&mut bits, child);
                        }
                        later |= get(&of, child) && self.named_ok(child, *named);
                    }
                }
                bits
            }
            Set::Chain { named, through, to } => {
                // In reverse preorder a node's children come before it: `has` is complete when the
                // node is reached.
                let (through, to) = (self.eval(through), self.eval(to));
                let mut has = vec![0_u64; words];
                for node in (0..len as u32).rev() {
                    let ends = get(&to, node) || (get(&through, node) && get(&has, node));
                    if ends
                        && self.named_ok(node, *named)
                        && let Some(parent) = self.tape.transducer_parent(node)
                    {
                        put(&mut has, parent);
                    }
                }
                has
            }
            Set::Reached { step, named, from } => {
                let from = self.eval(from);
                let mut bits = vec![0_u64; words];
                match step {
                    Step::Child => {
                        for node in ones(&from) {
                            for child in self.tape.transducer_children(node) {
                                if self.named_ok(child, *named) {
                                    put(&mut bits, child);
                                }
                            }
                        }
                    }
                    Step::Descendant => {
                        // A member inside one already spread adds nothing.
                        let mut covered = 0;
                        for node in ones(&from) {
                            let range = self.tape.transducer_descendants(node);
                            for below in range.start.max(covered)..range.end {
                                if self.named_ok(below, *named) {
                                    put(&mut bits, below);
                                }
                            }
                            covered = covered.max(range.end);
                        }
                    }
                    Step::Parent => {
                        for node in ones(&from) {
                            if let Some(parent) = self.tape.transducer_parent(node) {
                                put(&mut bits, parent);
                            }
                        }
                    }
                    Step::Following => {
                        let mut parents = vec![0_u64; words];
                        for node in ones(&from) {
                            let Some(parent) = self.tape.transducer_parent(node) else { continue };
                            if get(&parents, parent) {
                                continue;
                            }
                            put(&mut parents, parent);
                            let mut after = false;
                            for child in self.tape.transducer_children(parent) {
                                if after && self.named_ok(child, *named) {
                                    put(&mut bits, child);
                                }
                                after |= get(&from, child);
                            }
                        }
                    }
                    Step::Chain(through) => {
                        // In preorder a node's parent comes before it.
                        let through = self.eval(through);
                        for node in 0..len as u32 {
                            let Some(parent) = self.tape.transducer_parent(node) else { continue };
                            if self.named_ok(node, *named) && (get(&from, parent) || (get(&bits, parent) && get(&through, parent))) {
                                put(&mut bits, node);
                            }
                        }
                    }
                }
                bits
            }
        }
    }

    fn named_ok(&self, node: u32, named: bool) -> bool {
        !named || self.flags()[node as usize] & NAMED != 0
    }
}

fn get(bits: &[u64], node: u32) -> bool {
    bits[node as usize / 64] >> (node % 64) & 1 != 0
}

fn put(bits: &mut [u64], node: u32) {
    bits[node as usize / 64] |= 1 << (node % 64);
}

fn ones(words: &[u64]) -> impl Iterator<Item = u32> + '_ {
    words.iter().enumerate().flat_map(|(word, &bits)| {
        let mut bits = bits;
        std::iter::from_fn(move || {
            (bits != 0).then(|| {
                let bit = bits.trailing_zeros();
                bits &= bits - 1;
                word as u32 * 64 + bit
            })
        })
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn one_step_programs_are_formulas() {
        let w = |op, arg| word(op, 0, arg);
        let label = DERIVED_CLASS_BASE + 3;
        // "Some descendant has the label": the label's members' ancestors.
        let has = [w(SAVE_CURSOR, 0), w(DESCEND, 0), w(SUPERTYPE, label), w(RESTORE_CURSOR, 0), w(CAPTURE, 0), w(EMIT, 1)];
        assert_eq!(Set::of(&has), Some(Set::Descendant { named: false, of: Box::new(Set::Class(label)) }));
        // `FORK 4; KIND 1; GOTO 5; KIND 2; SUPERTYPE L`: (kind 1 or kind 2) and the label.
        let either = [w(FORK, 3), w(KIND, 1), w(GOTO, 4), w(KIND, 2), w(SUPERTYPE, label), w(EMIT, 1)];
        assert_eq!(Set::of(&either), Some(Set::And(vec![Set::Or(vec![Set::Kind(1), Set::Kind(2)]), Set::Class(label)])));
        // A branch that moves and does not come back is not a formula.
        let moved = [w(FORK, 3), w(CHILD, 0), w(GOTO, 4), w(KIND, 2), w(KIND, 5), w(EMIT, 1)];
        assert_eq!(Set::of(&moved), None);
        // A next-sibling step is not one.
        assert_eq!(Set::of(&[w(NEXT, 0), w(KIND, 1), w(EMIT, 1)]), None);
        // A later sibling is.
        assert_eq!(Set::of(&[w(FOLLOWING, 0), w(KIND, 1), w(EMIT, 1)]), Some(Set::Following { named: false, of: Box::new(Set::Kind(1)) }));
        // `CHILD; FORK 4; SUPERTYPE L; GOTO 0; KIND 2`: a chain of children with the label, ending at
        // one of kind 2.
        let chain = [w(CHILD, 0), w(FORK, 4), w(SUPERTYPE, label), w(GOTO, 0), w(KIND, 2), w(EMIT, 1)];
        assert_eq!(Set::of(&chain), Some(Set::Chain { named: false, through: Box::new(Set::Class(label)), to: Box::new(Set::Kind(2)) }));
    }

    #[test]
    fn captures_at_the_end_are_images() {
        let w = |op, arg| word(op, 0, arg);
        let label = DERIVED_CLASS_BASE + 3;
        // `inside` a labelled node: the descendants of its members.
        let inside = [w(SUPERTYPE, label), w(DESCEND, 0), w(CAPTURE, 0), w(EMIT, 1)];
        assert_eq!(
            Set::targets(&inside, Set::All),
            Some(Set::Reached { step: Step::Descendant, named: false, from: Box::new(Set::Class(label)) })
        );
        // A chain's every node from the starts of kind 7.
        let chain = [w(CHILD, 0), w(FORK, 4), w(SUPERTYPE, label), w(GOTO, 0), w(CAPTURE, 0), w(EMIT, 1)];
        assert_eq!(
            Set::targets(&chain, Set::Kind(7)),
            Some(Set::Reached { step: Step::Chain(Box::new(Set::Class(label))), named: false, from: Box::new(Set::Kind(7)) })
        );
        // A capture before a test is not an image.
        assert_eq!(Set::targets(&[w(CAPTURE, 0), w(KIND, 1), w(CAPTURE, 0), w(EMIT, 1)], Set::All), None);
    }
}
