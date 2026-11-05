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
//! - `SAVE … RESTORE` scopes a step: the tests after it read the start again.
//!
//! A program outside this fragment (a sibling step, a repetition, a capture other than the root's,
//! a branch that moves and does not come back) is not a formula, and runs as a walk.

use super::*;
use crate::code_rule_dsl::tape::{DERIVED_CLASS_BASE, TapeDoc};
use std::collections::HashMap;
use std::sync::Mutex;

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
            // The rest of the region is read at the node the step reaches.
            CHILD | DESCEND | PARENT => {
                let named = mode == NAMED;
                let of = Box::new(parse(words, pc + 1, to)?.0);
                parts.push(match op {
                    CHILD => Set::Child { named, of },
                    DESCEND => Set::Descendant { named, of },
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

/// A tape's columns as sets, built once per labelling: kind and field postings, flags.
pub(in crate::code_rule_dsl) struct Index {
    kinds: HashMap<u16, Vec<u32>>,
    fields: HashMap<u16, Vec<u32>>,
    flags: Vec<u8>,
    /// Grammar supertypes asked for, by class.
    classes: Mutex<HashMap<u16, Arc<Vec<u64>>>>,
}

impl Index {
    pub(in crate::code_rule_dsl) fn of(tape: &TapeDoc) -> Self {
        let len = tape.transducer_len() as u32;
        let mut kinds = HashMap::<u16, Vec<u32>>::new();
        let mut fields = HashMap::<u16, Vec<u32>>::new();
        let mut flags = Vec::with_capacity(len as usize);
        for node in 0..len {
            kinds.entry(tape.transducer_kind(node)).or_default().push(node);
            fields.entry(tape.transducer_field(node)).or_default().push(node);
            flags.push(tape.transducer_flags(node));
        }
        Self { kinds, fields, flags, classes: Default::default() }
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

    fn postings(&self, nodes: Option<&Vec<u32>>) -> Vec<u64> {
        let mut bits = vec![0_u64; self.words()];
        for &node in nodes.map_or(&[][..], Vec::as_slice) {
            bits[node as usize / 64] |= 1 << (node % 64);
        }
        bits
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
            Set::Kind(kind) => self.postings(self.index.kinds.get(kind)),
            Set::Field(field) => self.postings(self.index.fields.get(field)),
            Set::Flags { mode, arg } => {
                let mut bits = vec![0_u64; words];
                for (node, &flags) in self.index.flags.iter().enumerate() {
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
                    if *named && self.index.flags[node as usize] & NAMED == 0 {
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
                    if *named && self.index.flags[node as usize] & NAMED == 0 {
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
        }
    }
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
mod tests {
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
        // A sibling step is not one.
        assert_eq!(Set::of(&[w(NEXT, 0), w(KIND, 1), w(EMIT, 1)]), None);
    }
}
