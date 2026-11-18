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
pub enum Set {
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

/// A walk's anchor as the device evaluates it (`label_formula`, decision 0365): `Set` in postfix over a stack of node
/// sets, a word an operation (`op | arg << 8`), `depth` the stack's slots (an axis step's output a slot past its
/// input), and the labels it reads (by index, past `DERIVED_CLASS_BASE`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeviceFormula {
    pub words: Vec<u32>,
    pub depth: u32,
    pub labels: Vec<u32>,
    /// A text test's row by its slot (`TEXT`'s `arg`): each slot's key (`Micro::slot_key`), set by whoever lowered
    /// the walk; the device reads the row the walk reads (every node where there is none, for an anchor). Part of
    /// the formula's identity: two walks' slot 0 are different texts.
    pub texts: Vec<String>,
}

/// `DeviceFormula`'s operations (`label_formula` in `metal_label_passes.metal`). A test pushes a set; `AND` and
/// `OR` combine the top `arg` sets; an axis step replaces the top set (a chain the top two: through, then to); `arg`
/// of a step is whether its nodes are named.
pub mod device_ops {
    pub const ALL: u32 = 0;
    pub const KIND: u32 = 1;
    pub const FIELD: u32 = 2;
    /// `arg`: mode | flags << 8.
    pub const FLAGS: u32 = 3;
    /// A grammar supertype, or a label (from `DERIVED_CLASS_BASE`; on the device, `DERIVED_CLASS_BASE` + its row).
    pub const CLASS: u32 = 4;
    pub const AND: u32 = 5;
    pub const OR: u32 = 6;
    pub const CHILD: u32 = 7;
    pub const DESCENDANT: u32 = 8;
    pub const PARENT: u32 = 9;
    pub const FOLLOWING: u32 = 10;
    pub const CHAIN: u32 = 11;
    pub const NONE: u32 = 12;
    /// On the device only (`Runner::formulas`): a leaf test's shared row, `arg` its index.
    pub const LEAF: u32 = 13;
    /// A step within a conjunction: the top two sets, the conjunction's and the step's inner, as their intersection
    /// with the step's (`arg` named).
    pub const CHILD_IN: u32 = 14;
    pub const DESCENDANT_IN: u32 = 15;
    pub const FOLLOWING_IN: u32 = 16;
    /// A step over every node: the nodes with a next sibling, a node below (a child), a named child, a next named
    /// sibling.
    pub const HAS_NEXT: u32 = 18;
    pub const HAS_BELOW: u32 = 19;
    pub const HAS_NAMED_CHILD: u32 = 21;
    pub const HAS_NEXT_NAMED: u32 = 22;
    /// A text test: `arg` its slot (`DeviceFormula::texts`); on the device, its row in the bound text rows.
    pub const TEXT: u32 = 20;
    /// A targets image (`Set::Reached`): the top set's members' proper descendants, their later siblings, or (the top
    /// two: from, through) a chain's reach, unnamed (a named step is the image and `FLAGS` named). A child step is
    /// `PARENT` (the nodes whose parent is a member), a parent step `CHILD` unnamed (the nodes with a member child).
    pub const REACH_DESCENDANT: u32 = 23;
    pub const REACH_FOLLOWING: u32 = 24;
    pub const REACH_CHAIN: u32 = 25;
}

/// A step from a node, as a targets label's image follows it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Step {
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
    pub fn of(words: &[u32]) -> Option<Self> {
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
    pub fn targets(words: &[u32], starts: Self) -> Option<Self> {
        let end = words.len().checked_sub(2)?;
        if split(words[end + 1]).0 != EMIT || split(words[end]).0 != CAPTURE || words[..end].iter().any(|&word| split(word).0 == CAPTURE) {
            return None;
        }
        forward(words, 0, end, starts)
    }

    /// A walk's anchor (decision 0365): a superset of the starts from which `words` reaches its `EMIT`, for any
    /// program, a formula or not. Its tests are a conjunction read as `of` reads them, and where the program leaves
    /// the fragment (a capture is skipped; a next-sibling step, a backward jump, a repetition or any other operation
    /// ends it) the rest is dropped: a conjunct fewer is a larger set, and every step's set grows with what it reads
    /// at the node it reaches, so the anchor holds every start that emits. A `TEXT` operand is a text slot
    /// (`Micro`'s words), read where `texts` has its row, and dropped otherwise.
    pub fn anchor(words: &[u32], texts: &dyn Fn(u16) -> bool) -> Self {
        let end = words.iter().position(|&word| split(word).0 == EMIT).unwrap_or(words.len());
        bound(words, 0, end, texts).0
    }

    /// This set as the device evaluates it (decision 0365). Exact, so a formula pass may run as it (`Pass::Formula`), a
    /// roots pass's or a targets pass's image (`Reached`): a text test reads its slot's row (`DeviceFormula::texts`,
    /// set by the caller), and only an anchor reads every node where there is none.
    pub fn device(&self) -> Option<DeviceFormula> {
        let mut formula = DeviceFormula { words: Vec::new(), depth: 0, labels: Vec::new(), texts: Vec::new() };
        formula.depth = self.emit(&mut formula)?;
        formula.labels.sort_unstable();
        formula.labels.dedup();
        Some(formula)
    }

    /// Appends this set's postfix words: the stack slots its evaluation takes from where it starts.
    fn emit(&self, formula: &mut DeviceFormula) -> Option<u32> {
        use device_ops::*;
        let word = |op: u32, arg: u32| op | arg << 8;
        let named = |named: &bool| u32::from(*named);
        let slots = match self {
            Set::All => {
                formula.words.push(word(ALL, 0));
                1
            }
            Set::Text(slot) => {
                formula.words.push(word(TEXT, u32::from(*slot)));
                1
            }
            Set::Kind(kind) => {
                formula.words.push(word(KIND, u32::from(*kind)));
                1
            }
            Set::Field(field) => {
                formula.words.push(word(FIELD, u32::from(*field)));
                1
            }
            Set::Flags { mode, arg } => {
                formula.words.push(word(FLAGS, u32::from(*mode) | u32::from(*arg) << 8));
                1
            }
            Set::Class(class) => {
                if *class >= DERIVED_CLASS_BASE {
                    formula.labels.push(u32::from(class - DERIVED_CLASS_BASE));
                }
                formula.words.push(word(CLASS, u32::from(*class)));
                1
            }
            Set::And(parts) | Set::Or(parts) if parts.is_empty() => {
                formula.words.push(word(if matches!(self, Set::And(_)) { ALL } else { NONE }, 0));
                1
            }
            // A conjunction of tests and steps: the tests' set first, then each child, descendant or following step
            // within it (`CHILD_IN`, ...), so the device may read down from the conjunction's members instead of
            // climbing from the step's (S277: climb against descent priced by work, on the device by the members).
            Set::And(parts) if parts.iter().any(Set::within) && parts.iter().any(|part| !part.within()) => {
                let plain: Vec<&Set> = parts.iter().filter(|part| !part.within()).collect();
                let mut slots = 0;
                for (at, part) in plain.iter().enumerate() {
                    slots = slots.max(at as u32 + part.emit(formula)?);
                }
                if plain.len() > 1 {
                    formula.words.push(word(AND, plain.len() as u32));
                }
                for part in parts.iter().filter(|part| part.within()) {
                    let (Set::Child { named: n, of } | Set::Descendant { named: n, of } | Set::Following { named: n, of }) = part else {
                        unreachable!("a step within")
                    };
                    let op = match part {
                        Set::Child { .. } => CHILD_IN,
                        Set::Descendant { .. } => DESCENDANT_IN,
                        _ => FOLLOWING_IN,
                    };
                    // The conjunction's set, the step's, then its output and (a descendant's) its members and their
                    // counts' prefix.
                    let inner = of.emit(formula)?;
                    formula.words.push(word(op, named(n)));
                    slots = slots.max(1 + inner.max(match op {
                        DESCENDANT_IN => 4,
                        FOLLOWING_IN => 3,
                        _ => 2,
                    }));
                }
                slots
            }
            Set::And(parts) | Set::Or(parts) => {
                let mut slots = 0;
                for (at, part) in parts.iter().enumerate() {
                    slots = slots.max(at as u32 + part.emit(formula)?);
                }
                formula.words.push(word(if matches!(self, Set::And(_)) { AND } else { OR }, parts.len() as u32));
                slots
            }
            // A step over every node, by the node's own links, so no climb: a node below (in preorder, the same as a
            // child), a named child, a next sibling or a next named one. A named node below is the general step.
            Set::Child { .. } | Set::Descendant { .. } | Set::Following { .. } if self.over_all() => {
                let op = match self {
                    Set::Child { named: true, .. } => HAS_NAMED_CHILD,
                    Set::Child { .. } | Set::Descendant { .. } => HAS_BELOW,
                    Set::Following { named: true, .. } => HAS_NEXT_NAMED,
                    _ => HAS_NEXT,
                };
                formula.words.push(word(op, 0));
                1
            }
            Set::Child { named: n, of } | Set::Descendant { named: n, of } | Set::Following { named: n, of } => {
                let slots = of.emit(formula)?;
                let op = match self {
                    Set::Child { .. } => CHILD,
                    Set::Descendant { .. } => DESCENDANT,
                    _ => FOLLOWING,
                };
                formula.words.push(word(op, named(n)));
                // A following step's members' parents take a slot past its output.
                slots.max(if op == FOLLOWING { 3 } else { 2 })
            }
            Set::Parent(of) => {
                let slots = of.emit(formula)?;
                formula.words.push(word(PARENT, 0));
                slots.max(2)
            }
            Set::Chain { named: n, through, to } => {
                let through = through.emit(formula)?;
                let to = to.emit(formula)?;
                formula.words.push(word(CHAIN, named(n)));
                through.max(1 + to).max(3)
            }
            // A targets image (0365 §10): a child step is the nodes whose parent is a member, a parent step the nodes
            // with a member child; the others are steps of their own. A named step (its parent step ignores `named`,
            // as the CPU's) is the unnamed image within the named nodes, a chain's `through` too: a node the chain
            // goes on from is named.
            Set::Reached { step, named, from } => {
                let named = *named && !matches!(step, Step::Parent);
                let is_named = Set::Flags { mode: NAMED, arg: 0 };
                let slots = match step {
                    Step::Child => Set::Parent(from.clone()).emit(formula)?,
                    Step::Parent => Set::Child { named: false, of: from.clone() }.emit(formula)?,
                    Step::Descendant | Step::Following => {
                        let slots = from.emit(formula)?;
                        formula.words.push(word(if matches!(step, Step::Descendant) { REACH_DESCENDANT } else { REACH_FOLLOWING }, 0));
                        slots.max(2)
                    }
                    Step::Chain(through) => {
                        let slots = from.emit(formula)?;
                        let through = if named { Set::and(vec![(**through).clone(), is_named.clone()]) } else { (**through).clone() };
                        let through = through.emit(formula)?;
                        formula.words.push(word(REACH_CHAIN, 0));
                        slots.max(1 + through).max(3)
                    }
                };
                if named {
                    is_named.emit(formula)?;
                    formula.words.push(word(AND, 2));
                }
                slots.max(2)
            }
        };
        Some(slots)
    }

    /// This set with its text slots `by` further on: a member's tests in a union whose slots are its members' in turn
    /// (a fused pack's anchor, decision 0365).
    pub fn shift_texts(self, by: u16) -> Self {
        let shift = |set: Box<Set>| Box::new(set.shift_texts(by));
        match self {
            Set::Text(slot) => Set::Text(slot + by),
            Set::And(parts) => Set::And(parts.into_iter().map(|part| part.shift_texts(by)).collect()),
            Set::Or(parts) => Set::Or(parts.into_iter().map(|part| part.shift_texts(by)).collect()),
            Set::Child { named, of } => Set::Child { named, of: shift(of) },
            Set::Descendant { named, of } => Set::Descendant { named, of: shift(of) },
            Set::Following { named, of } => Set::Following { named, of: shift(of) },
            Set::Parent(of) => Set::Parent(shift(of)),
            Set::Chain { named, through, to } => Set::Chain { named, through: shift(through), to: shift(to) },
            Set::Reached { step, named, from } => {
                let step = match step {
                    Step::Chain(through) => Step::Chain(shift(through)),
                    step => step,
                };
                Set::Reached { step, named, from: shift(from) }
            }
            set @ (Set::All | Set::Kind(_) | Set::Field(_) | Set::Flags { .. } | Set::Class(_)) => set,
        }
    }

    /// A step a conjunction evaluates within its tests' set on the device (`DeviceFormula`).
    fn within(&self) -> bool {
        matches!(self, Set::Child { .. } | Set::Descendant { .. } | Set::Following { .. }) && !self.over_all()
    }

    /// A step over every node the device reads as a leaf (`HAS_BELOW`, `HAS_NAMED_CHILD`, `HAS_NEXT`, `HAS_NEXT_NAMED`).
    fn over_all(&self) -> bool {
        match self {
            Set::Child { of, .. } | Set::Following { of, .. } | Set::Descendant { named: false, of } => **of == Set::All,
            _ => false,
        }
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

/// `anchor`'s superset of the starts from which `words[from..to]` goes on, and whether the code leaves the node
/// moved (an axis step) or ends early (where it would go on is not known), where `parse` has the exact set or
/// nothing.
fn bound(words: &[u32], from: usize, to: usize, texts: &dyn Fn(u16) -> bool) -> (Set, bool) {
    let mut parts = Vec::new();
    let mut pc = from;
    while pc < to {
        let (op, mode, arg) = split(words[pc]);
        match op {
            KIND => parts.push(Set::Kind(arg)),
            FIELD => parts.push(Set::Field(arg)),
            FLAGS => match u8::try_from(arg) {
                Ok(arg) => parts.push(Set::Flags { mode, arg }),
                Err(_) => break,
            },
            SUPERTYPE => parts.push(Set::Class(arg)),
            TEXT if texts(arg) => parts.push(Set::Text(arg)),
            TEXT | CAPTURE => {}
            SAVE_CURSOR => {
                let Some(restore) = (pc + 1..to).find(|&at| matches!(split(words[at]), (RESTORE_CURSOR, _, cursor) if cursor == arg)) else { break };
                parts.push(bound(words, pc + 1, restore, texts).0);
                pc = restore + 1;
                continue;
            }
            CHILD if let Some((through, after)) = chain(words, pc, to) => {
                let (through, moved) = bound(words, pc + 2, through, texts);
                let to = Box::new(bound(words, after, to, texts).0);
                // A chain whose step body moves is not the formula's chain: its first child at least.
                parts.push(if moved { Set::Child { named: mode == NAMED, of: Box::new(Set::All) } } else { Set::Chain { named: mode == NAMED, through: Box::new(through), to } });
                return (Set::and(parts), true);
            }
            CHILD | DESCEND | PARENT | FOLLOWING => {
                let named = mode == NAMED;
                let of = Box::new(bound(words, pc + 1, to, texts).0);
                parts.push(match op {
                    CHILD => Set::Child { named, of },
                    DESCEND => Set::Descendant { named, of },
                    FOLLOWING => Set::Following { named, of },
                    _ => Set::Parent(of),
                });
                return (Set::and(parts), true);
            }
            FORK => {
                let at = usize::from(arg);
                let join = (at > pc + 1 && at <= to).then(|| split(words[at - 1])).filter(|&(goto, _, join)| goto == GOTO && usize::from(join) >= at && usize::from(join) <= to);
                let Some((_, _, join)) = join else { break };
                let join = usize::from(join);
                let (first, moved_first) = bound(words, pc + 1, at - 1, texts);
                let (second, moved_second) = bound(words, at, join, texts);
                parts.push(Set::Or(vec![first, second]));
                // A branch that moved goes on elsewhere: the rest is not read at this node.
                if moved_first || moved_second {
                    return (Set::and(parts), true);
                }
                pc = join;
                continue;
            }
            _ => break,
        }
        pc += 1;
    }
    // Ended before `to`: where the rest would have been read is not known, so it is as if moved.
    (Set::and(parts), pc < to)
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
pub struct Index {
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
pub struct Reader<'a> {
    pub tape: &'a TapeDoc,
    pub index: &'a Index,
    /// The labels so far, by class less `DERIVED_CLASS_BASE`.
    pub labels: &'a [Vec<u64>],
    /// The program's text rows, by slot.
    pub texts: &'a [Vec<u64>],
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
    pub fn eval(&self, set: &Set) -> Vec<u64> {
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
                // One pass in preorder, each node's parent the subtree open around it: no parent is searched for.
                for (node, parent) in (0..self.tape.transducer_len() as u32).zip(self.tape.transducer_parents()) {
                    let Some(parent) = parent else { continue };
                    if !get(&of, node) || (*named && self.flags()[node as usize] & NAMED == 0) {
                        continue;
                    }
                    put(&mut bits, parent);
                }
                bits
            }
            Set::Descendant { named, of } => {
                let of = self.eval(of);
                let mut bits = vec![0_u64; words];
                // One pass in preorder: a member marks the subtrees open around it (its ancestors), up to the first
                // already marked, whose own ancestors are.
                let mut open: Vec<(u32, u32)> = Vec::new();
                for node in 0..self.tape.transducer_len() as u32 {
                    while open.last().is_some_and(|&(_, end)| end <= node) {
                        open.pop();
                    }
                    if get(&of, node) && !(*named && self.flags()[node as usize] & NAMED == 0) {
                        for &(ancestor, _) in open.iter().rev() {
                            if get(&bits, ancestor) {
                                break;
                            }
                            put(&mut bits, ancestor);
                        }
                    }
                    open.push((node, self.tape.transducer_descendants(node).end));
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

    /// Decision 0365: a walk's anchor is its formula where it is one, and otherwise the conjunction it reads before
    /// it leaves the fragment.
    #[test]
    fn an_anchor_is_the_formula_or_a_superset_of_it() {
        let w = |op, arg| word(op, 0, arg);
        let label = DERIVED_CLASS_BASE + 3;
        let has = [w(SAVE_CURSOR, 0), w(DESCEND, 0), w(SUPERTYPE, label), w(RESTORE_CURSOR, 0), w(CAPTURE, 0), w(EMIT, 1)];
        assert_eq!(Some(Set::anchor(&has, &|_| true)), Set::of(&has));
        // A root walk's captures are not tests: kind 4 with a child of kind 5, its captures skipped.
        let captured = [w(KIND, 4), w(CAPTURE, 0), w(SAVE_CURSOR, 0), w(CHILD, 0), w(KIND, 5), w(CAPTURE, 1), w(RESTORE_CURSOR, 0), w(CAPTURE, 2), w(EMIT, 1)];
        assert_eq!(Set::of(&captured), None);
        assert_eq!(Set::anchor(&captured, &|_| true), Set::And(vec![Set::Kind(4), Set::Child { named: false, of: Box::new(Set::Kind(5)) }]));
        // A next-sibling step ends its region: what the region reads past it is dropped, and the tests after the
        // cursor's restore are read at the start again.
        let next = [w(KIND, 1), w(SAVE_CURSOR, 0), w(NEXT, 0), w(KIND, 2), w(RESTORE_CURSOR, 0), w(KIND, 3), w(EMIT, 1)];
        assert_eq!(Set::anchor(&next, &|_| true), Set::And(vec![Set::Kind(1), Set::Kind(3)]));
        // Unscoped, it ends the walk.
        assert_eq!(Set::anchor(&[w(KIND, 1), w(NEXT, 0), w(KIND, 2), w(EMIT, 1)], &|_| true), Set::Kind(1));
        // A branch that moves: the union of the branches, and the rest (read elsewhere on one of them) dropped.
        let moved = [w(FORK, 3), w(CHILD, 0), w(GOTO, 4), w(KIND, 2), w(KIND, 5), w(EMIT, 1)];
        assert_eq!(Set::anchor(&moved, &|_| true), Set::Or(vec![Set::Child { named: false, of: Box::new(Set::All) }, Set::Kind(2)]));
        // As the device has it: postfix, with the stack's slots (an axis step's output past its input).
        use device_ops as d;
        let formula = Set::anchor(&captured, &|_| true).device().unwrap();
        assert_eq!(formula.words, vec![d::KIND | 4 << 8, d::KIND | 5 << 8, d::CHILD_IN]);
        assert_eq!(formula.depth, 3);
        let moved = Set::anchor(&moved, &|_| true).device().unwrap();
        assert_eq!((moved.words, moved.depth), (vec![d::HAS_BELOW, d::KIND | 2 << 8, d::OR | 2 << 8], 2));
        let labelled = Set::anchor(&has, &|_| true).device().unwrap();
        assert_eq!((labelled.words, labelled.depth, labelled.labels), (vec![d::CLASS | u32::from(label) << 8, d::DESCENDANT], 2, vec![3]));
        // A text test without its row is dropped.
        assert_eq!(Set::anchor(&[w(KIND, 1), w(TEXT, 0), w(EMIT, 1)], &|_| false), Set::Kind(1));
        assert_eq!(Set::anchor(&[w(KIND, 1), w(TEXT, 0), w(EMIT, 1)], &|_| true), Set::And(vec![Set::Kind(1), Set::Text(0)]));
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
        // As the device has them (0365 §10): the image's own steps, a named one within the named nodes, a child step as
        // the nodes whose parent is a member.
        use device_ops as d;
        let class = d::CLASS | u32::from(label) << 8;
        let image = Set::targets(&inside, Set::All).unwrap().device().unwrap();
        assert_eq!((image.words, image.depth), (vec![class, d::REACH_DESCENDANT], 2));
        let image = Set::targets(&chain, Set::Kind(7)).unwrap().device().unwrap();
        assert_eq!((image.words, image.depth), (vec![d::KIND | 7 << 8, class, d::REACH_CHAIN], 3));
        let named = [w(SUPERTYPE, label), word(CHILD, NAMED, 0), w(CAPTURE, 0), w(EMIT, 1)];
        let image = Set::targets(&named, Set::All).unwrap().device().unwrap();
        assert_eq!((image.words, image.depth), (vec![class, d::PARENT, d::FLAGS | u32::from(NAMED) << 8, d::AND | 2 << 8], 2));
    }
}
