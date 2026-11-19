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
use super::compile_micro::CLOSURE;
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

/// A walk's anchor as the device evaluates it (`label_phase`, decisions 0365, 0463): `Set` in postfix over a stack of node
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

/// `DeviceFormula`'s operations (`label_phase` in `label_passes.uglykl`, compiled by `phase_program`). A test pushes a set; `AND` and
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
                        // From, through, the reach, and the device's level (0437).
                        slots.max(1 + through).max(4)
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
        self.map_tests(&|test| match test {
            Set::Text(slot) => Set::Text(slot + by),
            test => test,
        })
    }

    /// This set with each read of a label `unknown` names as every node (decision 0365, a recursive group's anchor):
    /// a superset whatever those labels hold, since a set reads its labels only as members (a complement is a label
    /// of its own).
    pub fn labels_unknown(self, unknown: &dyn Fn(u16) -> bool) -> Self {
        self.map_tests(&|test| match test {
            Set::Class(class) if class >= DERIVED_CLASS_BASE && unknown(class - DERIVED_CLASS_BASE) => Set::All,
            test => test,
        })
    }

    /// This set with each label it reads (from `DERIVED_CLASS_BASE`) renamed by `map` (a clause's labels in its wave).
    pub fn relabelled(self, map: &dyn Fn(u16) -> u16) -> Self {
        self.map_tests(&|test| match test {
            Set::Class(class) if class >= DERIVED_CLASS_BASE => Set::Class(map(class)),
            test => test,
        })
    }

    /// This set with each text test read as every node: a superset, with no slot of the program it came from (a
    /// slot is its program's, so another walk's anchor would read its own row there; decision 0425).
    pub fn texts_dropped(self) -> Self {
        self.map_tests(&|test| match test {
            Set::Text(_) => Set::All,
            test => test,
        })
    }

    /// The labels this set reads (from `DERIVED_CLASS_BASE`), as their ids.
    pub fn labels(&self) -> Vec<u16> {
        let found = std::cell::RefCell::new(Vec::new());
        self.clone().map_tests(&|test| {
            if let Set::Class(class) = test
                && class >= DERIVED_CLASS_BASE
            {
                found.borrow_mut().push(class);
            }
            test
        });
        let mut found = found.into_inner();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The conjunction of `parts`, flattened (`All` dropped).
    pub fn all_of(parts: Vec<Self>) -> Self {
        Self::and(parts)
    }

    /// This set with `f` applied to each of its tests (the sets with no operand), its steps kept.
    fn map_tests(self, f: &dyn Fn(Set) -> Set) -> Self {
        let map = |set: Box<Set>| Box::new(set.map_tests(f));
        match self {
            Set::And(parts) => Set::and(parts.into_iter().map(|part| part.map_tests(f)).collect()),
            Set::Or(parts) => Set::or(parts.into_iter().map(|part| part.map_tests(f)).collect()),
            Set::Child { named, of } => Set::Child { named, of: map(of) },
            Set::Descendant { named, of } => Set::Descendant { named, of: map(of) },
            Set::Following { named, of } => Set::Following { named, of: map(of) },
            Set::Parent(of) => Set::Parent(map(of)),
            Set::Chain { named, through, to } => Set::Chain { named, through: map(through), to: map(to) },
            Set::Reached { step, named, from } => {
                let step = match step {
                    Step::Chain(through) => Step::Chain(map(through)),
                    step => step,
                };
                Set::Reached { step, named, from: map(from) }
            }
            test @ (Set::All | Set::Text(_) | Set::Kind(_) | Set::Field(_) | Set::Flags { .. } | Set::Class(_)) => f(test),
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

    /// A disjunction, nested ones flattened; every node where a part is (a label read as every node, `labels_unknown`).
    fn or(parts: Vec<Self>) -> Self {
        let mut flat = Vec::new();
        for part in parts {
            match part {
                Self::All => return Self::All,
                Self::Or(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        match flat.len() {
            1 => flat.pop().expect("one part"),
            _ => Self::Or(flat),
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
            CHILD | CLOSURE if let Some((from, through, after)) = chain(words, pc, to) => {
                let (through, _) = parse(words, from, through).filter(|(_, moved)| !moved)?;
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
            CHILD | CLOSURE if let Some((from, through, after)) = chain(words, pc, to) => {
                let (through, moved) = bound(words, from, through, texts);
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

/// `CHILD` at `pc` as a loop (or a micro walk's `CLOSURE`, which keeps the loop's other words), in either order of
/// its paths: `pc: CHILD; FORK a; ψ…; GOTO pc; a:` (the
/// path that goes on first), or `pc: CHILD; FORK on; GOTO a; on: ψ…; GOTO pc; a:` (the one that stops
/// first, `bounded`'s). Where it is one, `ψ`'s range and `a`, where the walk goes on from the node the
/// chain stops at.
fn chain(words: &[u32], pc: usize, to: usize) -> Option<(usize, usize, usize)> {
    let (fork, _, at) = split(*words.get(pc + 1)?);
    let at = usize::from(at);
    if fork != FORK || at <= pc + 2 || at > to {
        return None;
    }
    let (goto, _, back) = split(words[at - 1]);
    if goto == GOTO && usize::from(back) == pc {
        return Some((pc + 2, at - 1, at));
    }
    // `pc + 2` jumps past the loop, whose last word jumps back to `pc`.
    let (exit, _, after) = split(words[pc + 2]);
    let after = usize::from(after);
    if exit != GOTO || at != pc + 3 || after <= at || after > to {
        return None;
    }
    let (goto, _, back) = split(words[after - 1]);
    (goto == GOTO && usize::from(back) == pc).then_some((at, after - 1, after))
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
            CHILD | CLOSURE if let Some((from, through, after)) = chain(words, pc, to) => {
                let (through, _) = parse(words, from, through).filter(|(_, moved)| !moved)?;
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
                if goto != GOTO {
                    // `FORK a; steps…; a:`: the steps or none (`inside` through a field to `end`: the
                    // field child, or below it), both going on at `a`.
                    let stepped = forward(words, pc + 1, fork, at.clone())?;
                    at = Set::Or(vec![at, stepped]);
                    pc = fork;
                    continue;
                }
                if join < fork || join > to {
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
    /// Steps over every node asked for (`Reader::over_all`), by step.
    over_all: Mutex<HashMap<Set, Arc<Vec<u64>>>>,
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

    /// The members of `set`, a row of the tape's words.
    pub fn eval(&self, set: &Set) -> Vec<u64> {
        let mut bits = self.row(set).into_vec();
        bits.resize(self.words(), 0);
        bits
    }

    /// The members of `set`: a label's or text slot's row read where it is, a column set or a step over every node as
    /// the index keeps it, else made. A row may be shorter than the tape's words: the rest are 0 (`has`).
    fn row(&self, set: &Set) -> Row<'_> {
        match set {
            Set::Class(class) if *class >= DERIVED_CLASS_BASE => Row::Borrowed(self.labels.get(usize::from(class - DERIVED_CLASS_BASE)).map_or(&[], Vec::as_slice)),
            Set::Text(slot) => Row::Borrowed(self.texts.get(usize::from(*slot)).map_or(&[], Vec::as_slice)),
            Set::Class(class) => Row::Shared(self.class(*class)),
            Set::Child { of, .. } | Set::Descendant { of, .. } | Set::Following { of, .. } | Set::Parent(of) if **of == Set::All => Row::Shared(self.over_all(set)),
            Set::Or(parts) => {
                let mut bits = vec![0_u64; self.words()];
                for part in parts {
                    self.or_into(part, &mut bits);
                }
                Row::Owned(bits)
            }
            Set::And(parts) => Row::Owned(self.and(parts)),
            set => Row::Owned(self.made(set)),
        }
    }

    /// `bits` with `set`'s members added: a nested `or` and a column's postings into `bits` itself, no row of their own.
    fn or_into(&self, set: &Set, bits: &mut [u64]) {
        let postings = |nodes: &[u32], bits: &mut [u64]| nodes.iter().for_each(|&node| put(bits, node));
        match set {
            Set::Or(parts) => parts.iter().for_each(|part| self.or_into(part, bits)),
            Set::Kind(kind) => postings(self.kinds().get(*kind), bits),
            Set::Field(field) => postings(self.fields().get(*field), bits),
            set => {
                for (word, other) in bits.iter_mut().zip(self.row(set).iter()) {
                    *word |= other;
                }
            }
        }
    }

    /// A conjunction's members: its tests first, then its other parts, each step tested only at the members so far
    /// (`within`, as the device's `within`), until none is left.
    fn and(&self, parts: &[Set]) -> Vec<u64> {
        let rank = |part: &Set| match part {
            Set::All | Set::Kind(_) | Set::Field(_) | Set::Flags { .. } | Set::Class(_) | Set::Text(_) => 0,
            Set::Or(_) | Set::And(_) => 1,
            step if step.over_all() || matches!(step, Set::Parent(of) if **of == Set::All) => 1,
            _ => 2,
        };
        let mut order = parts.iter().collect::<Vec<_>>();
        order.sort_by_key(|part| rank(part));
        let Some((first, rest)) = order.split_first() else { return self.eval(&Set::All) };
        let mut bits = self.eval(first);
        for part in rest {
            if bits.iter().all(|&word| word == 0) {
                break;
            }
            if self.within(part, &mut bits) {
                continue;
            }
            let other = self.row(part);
            for (at, word) in bits.iter_mut().enumerate() {
                *word &= other.get(at).copied().unwrap_or(0);
            }
        }
        bits
    }

    /// `bits` within step `step`'s members, testing the step at each member of `bits` only: whether `step` is one
    /// tested so (a child, descendant or following-sibling step over a set that is not every node).
    fn within(&self, step: &Set, bits: &mut [u64]) -> bool {
        let (named, of) = match step {
            Set::Child { named, of } | Set::Descendant { named, of } | Set::Following { named, of } if **of != Set::All => (*named, self.row(of)),
            _ => return false,
        };
        let held = |node: u32| has(&of, node) && self.named_ok(node, named);
        let members = ones(bits).collect::<Vec<_>>();
        for node in members {
            let keep = match step {
                Set::Child { .. } => self.tape.transducer_children(node).any(held),
                Set::Descendant { .. } => {
                    let range = self.tape.transducer_descendants(node);
                    if named { members_in(&of, range.start, range.end).any(|below| self.named_ok(below, true)) } else { any_in(&of, range.start, range.end) }
                }
                _ => self.tape.transducer_parent(node).is_some_and(|parent| self.tape.transducer_children(parent).skip_while(|&child| child != node).skip(1).any(held)),
            };
            if !keep {
                bits[node as usize / 64] &= !(1 << (node % 64));
            }
        }
        true
    }

    fn kinds(&self) -> &Postings {
        let len = self.tape.transducer_len() as u32;
        self.index.kinds.get_or_init(|| Postings::of(len, |node| self.tape.transducer_kind(node)))
    }

    fn fields(&self) -> &Postings {
        let len = self.tape.transducer_len() as u32;
        self.index.fields.get_or_init(|| Postings::of(len, |node| self.tape.transducer_field(node)))
    }

    /// A grammar supertype's members, made once a labelling (`Index::classes`).
    fn class(&self, class: u16) -> Arc<Vec<u64>> {
        let mut classes = self.index.classes.lock().expect("class sets");
        Arc::clone(classes.entry(class).or_insert_with(|| {
            let mut bits = vec![0_u64; self.words()];
            for node in 0..self.tape.transducer_len() as u32 {
                if self.tape.transducer_has_supertype(node, class) {
                    put(&mut bits, node);
                }
            }
            Arc::new(bits)
        }))
    }

    /// A step over every node (a node with a child, a later sibling, a parent), made once a labelling
    /// (`Index::over_all`): a column of the tape's shape, which every anchor whose labels were read as every node asks.
    fn over_all(&self, step: &Set) -> Arc<Vec<u64>> {
        if let Some(bits) = self.index.over_all.lock().expect("steps over every node").get(step) {
            return Arc::clone(bits);
        }
        let bits = Arc::new(self.made(step));
        self.index.over_all.lock().expect("steps over every node").insert(step.clone(), Arc::clone(&bits));
        bits
    }

    /// The members of a test or step `set`, made over the tape.
    fn made(&self, set: &Set) -> Vec<u64> {
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
            Set::Kind(kind) => self.postings(self.kinds().get(*kind)),
            Set::Field(field) => self.postings(self.fields().get(*field)),
            Set::Flags { mode, arg } => {
                let mut bits = vec![0_u64; words];
                for (node, &flags) in self.flags().iter().enumerate() {
                    if flags & mode == *mode && flags & arg == 0 {
                        bits[node / 64] |= 1 << (node % 64);
                    }
                }
                bits
            }
            Set::Class(_) | Set::Text(_) | Set::And(_) | Set::Or(_) => self.eval(set),
            Set::Child { named, of } => {
                let of = self.row(of);
                let mut bits = vec![0_u64; words];
                // One pass in preorder, each node's parent the subtree open around it: no parent is searched for.
                for (node, parent) in (0..self.tape.transducer_len() as u32).zip(self.tape.transducer_parents()) {
                    let Some(parent) = parent else { continue };
                    if !has(&of, node) || (*named && self.flags()[node as usize] & NAMED == 0) {
                        continue;
                    }
                    put(&mut bits, parent);
                }
                bits
            }
            Set::Descendant { named, of } => {
                let of = self.row(of);
                let mut bits = vec![0_u64; words];
                // One pass in preorder: a member marks the subtrees open around it (its ancestors), up to the first
                // already marked, whose own ancestors are.
                let mut open: Vec<(u32, u32)> = Vec::new();
                for node in 0..self.tape.transducer_len() as u32 {
                    while open.last().is_some_and(|&(_, end)| end <= node) {
                        open.pop();
                    }
                    if has(&of, node) && !(*named && self.flags()[node as usize] & NAMED == 0) {
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
                let of = self.row(of);
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
                let of = self.row(of);
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
                        later |= has(&of, child) && self.named_ok(child, *named);
                    }
                }
                bits
            }
            Set::Chain { named, through, to } => {
                // In reverse preorder a node's children come before it: `has` is complete when the
                // node is reached.
                let (through, to) = (self.row(through), self.row(to));
                let mut reached = vec![0_u64; words];
                for node in (0..len as u32).rev() {
                    let ends = has(&to, node) || (has(&through, node) && get(&reached, node));
                    if ends
                        && self.named_ok(node, *named)
                        && let Some(parent) = self.tape.transducer_parent(node)
                    {
                        put(&mut reached, parent);
                    }
                }
                reached
            }
            Set::Reached { step, named, from } => {
                let from = self.row(from);
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
                                after |= has(&from, child);
                            }
                        }
                    }
                    Step::Chain(through) => {
                        // In preorder a node's parent comes before it.
                        let through = self.row(through);
                        for node in 0..len as u32 {
                            let Some(parent) = self.tape.transducer_parent(node) else { continue };
                            if self.named_ok(node, *named) && (has(&from, parent) || (get(&bits, parent) && has(&through, parent))) {
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

/// Whether `node` is in `bits`, a row that may be shorter than the tape's words (the rest are 0).
fn has(bits: &[u64], node: u32) -> bool {
    bits.get(node as usize / 64).is_some_and(|word| word >> (node % 64) & 1 != 0)
}

/// Whether any node of `from..to` is in `bits`: a word at a time.
fn any_in(bits: &[u64], from: u32, to: u32) -> bool {
    let (from, to) = (from as usize, (to as usize).min(bits.len() * 64));
    if from >= to {
        return false;
    }
    let (first, last) = (from / 64, (to - 1) / 64);
    let low = !0_u64 << (from % 64);
    let high = if to % 64 == 0 { !0_u64 } else { (1_u64 << (to % 64)) - 1 };
    if first == last {
        return bits[first] & low & high != 0;
    }
    bits[first] & low != 0 || bits[first + 1..last].iter().any(|&word| word != 0) || bits[last] & high != 0
}

/// The members of `bits` in `from..to`, a word at a time.
fn members_in(bits: &[u64], from: u32, to: u32) -> impl Iterator<Item = u32> + '_ {
    let to = to.min((bits.len() * 64) as u32);
    let words = if from < to { (from as usize / 64)..((to as usize - 1) / 64 + 1) } else { 0..0 };
    words.flat_map(move |at| {
        let mut word = bits[at];
        std::iter::from_fn(move || {
            (word != 0).then(|| {
                let bit = word.trailing_zeros();
                word &= word - 1;
                at as u32 * 64 + bit
            })
        })
    })
    .filter(move |&node| node >= from && node < to)
}

/// A set's members as `Reader::row` gives them: a row read where it is, one an index keeps, or one made.
enum Row<'a> {
    Borrowed(&'a [u64]),
    Shared(Arc<Vec<u64>>),
    Owned(Vec<u64>),
}

impl std::ops::Deref for Row<'_> {
    type Target = [u64];
    fn deref(&self) -> &[u64] {
        match self {
            Row::Borrowed(bits) => bits,
            Row::Shared(bits) => bits,
            Row::Owned(bits) => bits,
        }
    }
}

impl Row<'_> {
    /// The members as a row of their own: a made one as it is, a read one as the formula's answer.
    fn into_vec(self) -> Vec<u64> {
        match self {
            Row::Owned(bits) => bits,
            row => row.to_vec(),
        }
    }
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
        // The same chain with the path that stops first (`bounded`'s order, a closure on the device):
        // `CHILD; FORK 3; GOTO 5; SUPERTYPE L; GOTO 0; CAPTURE; EMIT`.
        let stops_first = [w(CHILD, 0), w(FORK, 3), w(GOTO, 5), w(SUPERTYPE, label), w(GOTO, 0), w(CAPTURE, 0), w(EMIT, 1)];
        assert_eq!(
            Set::targets(&stops_first, Set::Kind(7)),
            Some(Set::Reached { step: Step::Chain(Box::new(Set::Class(label))), named: false, from: Box::new(Set::Kind(7)) })
        );
        assert_eq!(
            Set::of(&[w(CHILD, 0), w(FORK, 3), w(GOTO, 5), w(SUPERTYPE, label), w(GOTO, 0), w(KIND, 2), w(EMIT, 1)]),
            Some(Set::Chain { named: false, through: Box::new(Set::Class(label)), to: Box::new(Set::Kind(2)) })
        );
        // `inside` through a field to `end`: from kind 7, the field child or below it.
        // `CHILD; FIELD 24; FORK 4; DESCEND; 4: CAPTURE; EMIT`.
        let field = [w(CHILD, 0), w(FIELD, 24), w(FORK, 4), w(DESCEND, 0), w(CAPTURE, 0), w(EMIT, 1)];
        let child = Set::and(vec![Set::Reached { step: Step::Child, named: false, from: Box::new(Set::Kind(7)) }, Set::Field(24)]);
        assert_eq!(
            Set::targets(&field, Set::Kind(7)),
            Some(Set::Or(vec![child.clone(), Set::Reached { step: Step::Descendant, named: false, from: Box::new(child) }]))
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
        assert_eq!((image.words, image.depth), (vec![d::KIND | 7 << 8, class, d::REACH_CHAIN], 4));
        let named = [w(SUPERTYPE, label), word(CHILD, NAMED, 0), w(CAPTURE, 0), w(EMIT, 1)];
        let image = Set::targets(&named, Set::All).unwrap().device().unwrap();
        assert_eq!((image.words, image.depth), (vec![class, d::PARENT, d::FLAGS | u32::from(NAMED) << 8, d::AND | 2 << 8], 2));
    }
}
