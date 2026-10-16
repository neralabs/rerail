//! The frame IR: what one statement computes, as operators from an input frame to an output frame.
//!
//! A frame is a set of rows `(k, file, node, slots…)`: `k` identifies the row, `node` is the node
//! the rule is evaluated at, and each slot holds a capture's binding (NULL where unbound). Every
//! operator maps each input row to at most one output row with the same `k`, `file` and `node`,
//! its slots updated with what the operator bound: the native matcher keeps one environment per
//! root and never backtracks, so this is exactly its semantics as set operations.

use std::fmt;

/// An index into a statement's slot table.
pub(super) type SlotId = usize;

/// What a capture binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum SlotKind {
    /// `$X`: one node id.
    Node,
    /// `$$$X`: a run of siblings `[lo, hi]` (`-1, -1` when bound to no node), the hash of its
    /// nodes' texts and how many of them are named.
    List,
    /// A count's `as`: an integer, rendered as text.
    Count,
}

/// A capture of one scope: the clause's own (scope 0) or a parameterized call's frame.
#[derive(Clone, Debug)]
pub(super) struct Slot {
    pub(super) name: String,
    pub(super) kind: SlotKind,
    pub(super) scope: usize,
}

impl Slot {
    /// The physical columns that hold the slot, in order.
    pub(super) fn columns(&self, id: SlotId) -> Vec<String> {
        match self.kind {
            SlotKind::Node | SlotKind::Count => vec![format!("s{id}")],
            SlotKind::List => vec![
                format!("s{id}a"),
                format!("s{id}b"),
                format!("s{id}h"),
                format!("s{id}n"),
            ],
        }
    }
}

/// An operator the SQL engine cannot lower yet. Compiling a clause that holds one fails with it;
/// the engine never answers for such a clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Unsupported(pub(in crate::code_rule_dsl) String);

impl fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Unsupported({})", self.0)
    }
}

/// How far a relation walks.
#[derive(Clone, Copy, Debug)]
pub(super) enum Stop {
    Neighbor,
    End,
    /// Until (and including) the first node of a shared stage: the stop rule, matched in a fresh
    /// environment, is a per-batch node set.
    Rule(usize),
}

/// A relation's walk from the frame's node: its candidates and their walk order.
#[derive(Clone, Copy, Debug)]
pub(super) enum Walk {
    Has(Stop),
    Inside(Stop),
    Precedes(Stop),
    Follows(Stop),
    /// `has` with a `field`: the first child in the field, then as the stop says.
    FieldHas(u16, Stop),
    /// `inside` with a `field`: ancestors entered through their first child in the field.
    FieldInside(u16, Stop),
}

impl Walk {
    /// Whether candidates are walked nearest-first in descending id order.
    pub(super) fn descending(self) -> bool {
        matches!(
            self,
            Self::Inside(_) | Self::Follows(_) | Self::FieldInside(..)
        )
    }
}

/// `nthChild`'s `ofRule`.
#[derive(Debug)]
pub(super) enum OfRule {
    /// A rule that reads or binds captures: probed at each named sibling with the row's bindings.
    Frame(Box<Op>),
    /// A capture-free rule: a per-batch stage.
    Stage(usize),
}

/// A parameterized call's copy of one caller capture into a parameter, and back.
#[derive(Debug)]
pub(super) struct CallSlot {
    pub(super) kind: SlotKind,
    pub(super) callee: SlotId,
    pub(super) caller: SlotId,
}

/// What a parameterized call returns under one caller name and kind: the caller's slot and every
/// parameter slot mapped to it, in argument order.
#[derive(Debug)]
pub(super) struct CallReturn {
    pub(super) kind: SlotKind,
    pub(super) caller: SlotId,
    pub(super) params: Vec<SlotId>,
}

#[derive(Debug)]
pub(super) enum Op {
    /// `node.kind IN (…)`.
    Kinds(Vec<u16>),
    /// The `text_match` terminal.
    Regex(usize),
    /// The `pattern` terminal. `env`: the env-input variant, given the frame's bindings of `reads`
    /// (the pattern's captures the frame may bind); otherwise fresh-env matches joined on the node.
    /// `vars` are the pattern's captures, in the terminal's column order.
    Pattern {
        term: usize,
        env: bool,
        vars: Vec<SlotId>,
        reads: Vec<SlotId>,
    },
    All(Vec<Op>),
    Any(Vec<Op>),
    Not(Box<Op>),
    Never,
    /// A relation: `binds` where the sub-rule may bind (the first match in walk order then binds;
    /// otherwise a semi-join).
    Relation {
        walk: Walk,
        sub: Box<Op>,
        binds: bool,
    },
    NthChild {
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: Option<OfRule>,
    },
    /// `count` over a relation's walk, two-valued; `bind` is the `as` slot and its offset.
    Count {
        walk: Walk,
        sub: Box<Op>,
        min: usize,
        max: Option<usize>,
        bind: Option<(SlotId, i64)>,
    },
    /// A semi-join with a shared stage: a context-free utility.
    Stage(usize),
    /// A parameterized call: `entry` copies caller captures into parameters, `body` runs in the
    /// callee's scope, `returns` brings back what it bound.
    Call {
        entry: Vec<CallSlot>,
        body: Box<Op>,
        returns: Vec<CallReturn>,
    },
    /// The query's `constraints`: after `rule`, each bound single-node capture matches its rule at
    /// the captured node, in order.
    Constrained {
        rule: Box<Op>,
        constraints: Vec<(SlotId, Op)>,
    },
}

impl Op {
    /// Whether the operator can bind a slot.
    pub(super) fn binds(&self) -> bool {
        match self {
            Self::Kinds(_) | Self::Regex(_) | Self::Never | Self::Not(_) | Self::Stage(_) => false,
            Self::Pattern { vars, .. } => !vars.is_empty(),
            Self::All(items) | Self::Any(items) => items.iter().any(Self::binds),
            Self::Relation { sub, .. } => sub.binds(),
            Self::NthChild { of_rule, .. } => match of_rule {
                Some(OfRule::Frame(rule)) => rule.binds(),
                _ => false,
            },
            Self::Count { bind, .. } => bind.is_some(),
            Self::Call { returns, .. } => !returns.is_empty(),
            Self::Constrained { .. } => true,
        }
    }
}

/// The candidate roots of a statement: nodes of `kinds` (every node without) in its files.
#[derive(Debug)]
pub(super) struct Statement {
    pub(super) slots: Vec<Slot>,
    pub(super) kinds: Option<Vec<u16>>,
    pub(super) op: Op,
}
