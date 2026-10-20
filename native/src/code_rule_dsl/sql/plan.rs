//! The frame IR: what one statement computes, as operators from an input frame to an output frame.
//!
//! A frame is a set of rows `(k, file, node, slots…)`: `k` identifies the row, `node` is the node
//! the rule is evaluated at, and each slot holds a capture's binding (NULL where unbound). Every
//! operator maps each input row to at most one output row with the same `k`, `file` and `node`,
//! its slots updated with what the operator bound: the native matcher keeps one environment per
//! root and never backtracks, so this is exactly its semantics as set operations.
//!
//! # Three-valued frames
//!
//! A clause the matcher runs three-valued (`GrammarMatcher::three_valued`) lowers under
//! `Op::Unknowable`, and every frame of its statement, and of the stages it lowers, carries more
//! columns after its slots: `unk` (0 or 1), then `wh0…whN`, the withheld-capture mask (62 bits a
//! column; no column where no `all` can withhold). Two-valued clauses have none of them and emit
//! exactly what they did before. Every operator keeps this contract, and so must every terminal
//! that reports an unknown verdict:
//!
//! - The output row of an operator is its verdict on the input row with the same `k`: no row is
//!   false; `unk = 0` is true, the slots holding what it bound; `unk = 1` is unknown, the slots
//!   holding what the matcher leaves on an unknown answer, which is the input row's bindings.
//! - An operator ignores the `unk` and `wh` of its input: they belong to the `all` whose pipeline
//!   the row is in. A row an operator emits as true has `wh = 0`.
//! - `Op::AllOpen` threads its own verdict through its frames in `unk` and the names it withholds
//!   in `wh`. Past an unknown term, a term whose mention mask meets `wh` passes the row on as it is
//!   and ORs its bind mask into `wh`; an unknown term that may bind ORs in its bind mask
//!   (`match_all`; the masks are `CaptureAnalysis`'s, one bit per capture name of the scope). The
//!   `all` is unknown where a term was, with its input's bindings.
//! - A terminal returns verdicts through `Op::Fact`: for each distinct input key, no row (false)
//!   or one row with its `unk` and the bindings it adds (`terminals::Verdicts`).
//! - A stage stores the nodes its rule is true or unknown at, with `unk`. A stop rule unknown at a
//!   node stops the walk there and leaves the relation open (`Search`'s `open`).
//! - A clause's matching roots are its output rows with `unk = 0`.

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
    /// A capture exported as text (`parse`'s and `formatArgs`' exports, `calls`' `chain`): its
    /// text, and its source range as the matcher's reserved span variable holds it, `start:end`.
    Text,
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
            SlotKind::Text => vec![format!("s{id}"), format!("s{id}r")],
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

/// A per-batch node set a statement reads: a stage's rows, or (`complement`) every node of the
/// batch's files but them. A capture-free rule `not R` is `R`'s stage complemented, so a set
/// holding most nodes of the batch is stored as the few it leaves out; every reader inverts
/// membership where `complement` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NodeSet {
    pub(super) stage: usize,
    pub(super) complement: bool,
}

impl NodeSet {
    /// The nodes this set leaves out.
    pub(super) fn complemented(self) -> Self {
        Self {
            stage: self.stage,
            complement: !self.complement,
        }
    }
}

/// How far a relation walks.
#[derive(Clone, Copy, Debug)]
pub(super) enum Stop {
    Neighbor,
    End,
    /// Until (and including) the first node of a per-batch set: the stop rule, matched in a
    /// fresh environment.
    Rule(NodeSet),
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
    /// The node a slot holds (in the row's file), as its one candidate: a terminal's related
    /// node (a binding's site, value or annotation) the frame carries.
    Slot(SlotId),
    /// A terminal's related nodes (`Statement::sources`), in the order it gives them.
    Related(usize),
}

impl Walk {
    /// Whether candidates are walked nearest-first in descending id order.
    pub(super) fn descending(self) -> bool {
        matches!(
            self,
            Self::Inside(_) | Self::Follows(_) | Self::FieldInside(..)
        )
    }

    /// Whether a row has one candidate at most, so a first match passes over none.
    pub(super) fn single(self) -> bool {
        matches!(
            self,
            Self::Inside(Stop::Neighbor)
                | Self::Precedes(Stop::Neighbor)
                | Self::Follows(Stop::Neighbor)
                | Self::FieldHas(_, Stop::Neighbor)
                | Self::FieldInside(_, Stop::Neighbor)
                | Self::Slot(_)
        )
    }
}

/// What a walk's candidate must be for its sub-rule to hold, or be unknown, at it, as far as no
/// capture decides (`lower::RootKinds`): the walk reads `kinds` from the kind postings, and
/// tests `shape` on each candidate before its row is built. Dropping a candidate that is not so
/// drops one the sub-rule is false at, which changes neither a first match nor a count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Need {
    /// Its kinds; `None`: any.
    pub(super) kinds: Option<Vec<u16>>,
    /// A structural test beyond its kinds; `None`: none.
    pub(super) shape: Option<Shape>,
}

/// A capture-free test on a node (`Need::shape`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Shape {
    /// Its kind is one of these.
    Kinds(Vec<u16>),
    /// A proper descendant's kind is one of these (`has` with `stopBy: end`).
    Contains(Vec<u16>),
    /// A child's kind is one of these (`has`).
    Child(Vec<u16>),
    All(Vec<Shape>),
    Any(Vec<Shape>),
}

impl Shape {
    /// Whether it tests more than a kind: kinds alone are what `Need::kinds` reads already.
    pub(super) fn structural(&self) -> bool {
        match self {
            Self::Kinds(_) => false,
            Self::Contains(_) | Self::Child(_) => true,
            Self::All(shapes) | Self::Any(shapes) => shapes.iter().any(Self::structural),
        }
    }
}

/// `nthChild`'s `ofRule`.
#[derive(Debug)]
pub(super) enum OfRule {
    /// A rule that reads or binds captures: probed at each named sibling with the row's bindings.
    Frame(Box<Op>),
    /// A capture-free rule: a per-batch set.
    Stage(NodeSet),
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

/// The candidates of a `Walk::Related`: for each distinct key of the frame, `(file, node if
/// at_node, bindings of reads)`, the fact terminal `term` returns rows `(file, node, ord, values…)`:
/// a candidate in `file` (the row's own where NULL), at `node`, walked in ascending `ord`, with
/// `values` written into `sets` on the candidate's row.
#[derive(Debug)]
pub(super) struct Source {
    pub(super) term: usize,
    pub(super) reads: Vec<SlotId>,
    pub(super) at_node: bool,
    pub(super) sets: Vec<SlotId>,
}

/// A withheld-capture mask: one word per `wh` column, 62 bits each.
pub(super) type Mask = Vec<i64>;

/// The bits of `bits` (capture indices of the statement's withholding) as a mask of `words`.
pub(super) fn mask(bits: impl IntoIterator<Item = usize>, words: usize) -> Mask {
    let mut mask = vec![0; words];
    for bit in bits {
        mask[bit / MASK_BITS] |= 1 << (bit % MASK_BITS);
    }
    mask
}

/// The bits one `wh` column holds.
pub(super) const MASK_BITS: usize = 62;

/// One term of a three-valued `all`, with what `match_all` reads of it to withhold.
#[derive(Debug)]
pub(super) struct AllTerm {
    pub(super) op: Op,
    /// The captures it mentions (`CaptureAnalysis::mentions`): past an unknown term that may bind
    /// one of them, it is withheld.
    pub(super) mentions: Mask,
    /// The captures it binds (`CaptureAnalysis::binds`): withheld from the terms after it where
    /// it is withheld, or where it is unknown and `withholds`.
    pub(super) binds: Mask,
    /// Whether its unknown verdict withholds `binds` (`!AstMatcher::binds_nothing`).
    pub(super) withholds: bool,
    /// Whether a row can reach it with a capture it mentions withheld; where none can, its rows
    /// are not split.
    pub(super) checked: bool,
}

/// A recursive utility's evaluation unit (`recursion` explains why it is exact): its answers are
/// remembered per key, `(file, node, bindings of key)`, in a demand table (the keys asked for) and
/// an answer table (the bindings after the body, for the keys it matched). The driver evaluates
/// the body over every demanded key, pass after pass, until no answer changes and no key is
/// added; a `Recur` reference reads the answers so far.
///
/// A unit of a cycle whose recursion goes through a binding's site, value or annotation
/// (`recursion::Cycle::following`) is keyed by its following context too (`following`): the
/// binding sub-rules being matched (`binding::FOLLOWING`, which the matcher's memo keys its
/// verdicts by), as an interned set (`Op::Follow`).
#[derive(Debug)]
pub(super) struct Unit {
    /// The slots the body reads and binds, in the unit's own scope: its key, and its answer.
    pub(super) key: Vec<SlotId>,
    pub(super) body: Op,
    /// The units that reach each other (a strongly connected component) share one loop.
    pub(super) scc: usize,
    /// The slot holding the following context its body starts from, where it is keyed by one.
    pub(super) following: Option<SlotId>,
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
    /// otherwise a semi-join). `need`: what a candidate must be for the sub-rule to hold there.
    Relation {
        walk: Walk,
        sub: Box<Op>,
        binds: bool,
        need: Need,
    },
    NthChild {
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: Option<OfRule>,
    },
    /// `count` over a relation's walk, two-valued; `bind` is the `as` slot and its offset. `need`
    /// as a relation's.
    Count {
        walk: Walk,
        sub: Box<Op>,
        min: usize,
        max: Option<usize>,
        bind: Option<(SlotId, i64)>,
        need: Need,
    },
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
    /// The root of a three-valued statement (the module doc's contract): frames carry `unk` and
    /// `masks` withheld-mask columns; `open` lists the stop-rule stages whose verdict may be
    /// unknown.
    Unknowable {
        op: Box<Op>,
        masks: usize,
        open: Vec<usize>,
    },
    /// A three-valued `all`: its terms in order, withholding as `match_all` does.
    AllOpen(Vec<AllTerm>),
    /// A three-valued `any`: each branch from the input bindings, the first true one wins. A
    /// branch whose verdict is unknown ends the search unknown where it may bind (the `bool`,
    /// `relation::may_bind`); otherwise later branches still decide.
    AnyOpen(Vec<(Op, bool)>),
    /// A three-valued relation (`relation::Search`): the first candidate in walk order that is
    /// true binds. An unknown candidate ends the search unknown where it is `strict` (its rule may
    /// bind); otherwise the search goes on, and is unknown where nothing matched. `binds` where the
    /// sub-rule may bind slots. `need` as a relation's: the sub-rule is false, not unknown, at a
    /// candidate that is not so.
    Search {
        walk: Walk,
        sub: Box<Op>,
        binds: bool,
        strict: bool,
        need: Need,
    },
    /// An input-set terminal's verdicts (`terminals::Verdicts`): each distinct key of the frame,
    /// `(file, node if at_node, bindings of reads)`, is evaluated once natively; the terminal
    /// returns the keys that are true or unknown, with the values it binds into `binds`.
    Fact {
        term: usize,
        reads: Vec<SlotId>,
        at_node: bool,
        binds: Vec<SlotId>,
    },
    /// A recursive unit's answers at the frame's keys: `slots` are the frame's slots for the
    /// unit's key, in order (`None`: the key's own slots, a shared utility in its caller's scope).
    /// `context`: the slot holding the following context the reference passes, where the unit is
    /// keyed by one and the reference lies in its cycle; elsewhere it passes none (`Op::Follow`).
    Recur {
        unit: usize,
        slots: Option<Vec<SlotId>>,
        context: Option<SlotId>,
    },
    /// A binding's site, value or annotation sub-rule `sub` at the row's node, as
    /// `binding::match_binding_nodes` guards it (`FOLLOWING`): false where the row's following
    /// context (the interned set in `base`, none where NULL) holds `(node, rule)` already; else
    /// `sub` runs with `out` holding the context with `(node, rule)` added. `rule` numbers the
    /// sub-rule in its statement, one number per matcher whatever the lowerings of it.
    Follow {
        rule: usize,
        base: SlotId,
        out: SlotId,
        sub: Box<Op>,
    },
    /// `violation.each`: the root a row is evaluated for, set in `root`, and on a later round of
    /// the enumeration only the roots still enumerated.
    EachRoot {
        root: SlotId,
    },
    /// `violation.each`: drops a row whose binding of the enumerated capture (`name`, in the
    /// current scope) has a range already reported for its root.
    Excluded {
        name: SlotId,
        root: SlotId,
    },
    /// `nthChild` with a binding `ofRule` while `violation.each` excludes: the position comes from
    /// `nth` (its probes exclude nothing), the bindings from `commit`, the rule matched again at
    /// the node excluding; where `commit` does not match, the position holds with no bindings.
    NthCommit {
        nth: Box<Op>,
        commit: Box<Op>,
    },
    /// Sub-rules matched one after another on one trial environment, as the matcher matches a
    /// binding's site, value and annotation (`binding::match_binding_nodes`) and the steps of the
    /// semantic predicates after them: the first that is not true decides, false or unknown.
    Seq(Vec<Op>),
    /// A terminal's verdict on a candidate before its sub-rules, which the slot holds
    /// (`matcher::PASSES`, `OPEN`, `FAILS`): true, unknown or false (NULL is true).
    Gate(SlotId),
    /// `redefines.every`: true where the sub-rule is true at every candidate of the walk; else
    /// the verdict of the first candidate in walk order where it is not, false or unknown.
    Every {
        walk: Walk,
        sub: Box<Op>,
    },
    /// `parse`: the tree terminal `term` parses from the row's node is a document of the batch's
    /// own (`exec::SubFile`), its number kept in `tree`; `body`, in the tree's own scope (its
    /// slots `inner`, unbound on entry), runs at its root, the host file and node kept in `host`.
    /// Where it matches, the exports copy inner captures' texts and host ranges into host text
    /// slots.
    Parse {
        term: usize,
        inner: Vec<SlotId>,
        host: (SlotId, SlotId),
        tree: SlotId,
        body: Box<Op>,
        exports: Vec<Export>,
    },
    /// `host` inside a parsed tree's rule: `rule` at the host node `host` holds, in the host's
    /// environment with the tree's captures so far exported into it (`exports`, from the tree
    /// `tree` holds); it binds nothing.
    Host {
        host: (SlotId, SlotId),
        tree: SlotId,
        rule: Box<Op>,
        exports: Vec<Export>,
    },
    /// Copies parsed captures' texts and host ranges, from the tree `tree` holds, into host text
    /// slots (`formatArgs.missing`'s exports).
    Exports {
        tree: SlotId,
        exports: Vec<Export>,
    },
}

/// A parsed tree's capture as its host reads it: the inner node slot `from`, whose text and host
/// range the fact terminal `term` gives, into the host's text slot `to`.
#[derive(Debug)]
pub(super) struct Export {
    pub(super) from: SlotId,
    pub(super) to: SlotId,
    pub(super) term: usize,
}

impl Op {
    /// Whether the operator can bind a slot.
    pub(super) fn binds(&self) -> bool {
        match self {
            Self::Kinds(_) | Self::Regex(_) | Self::Never | Self::Not(_) => false,
            Self::Unknowable { op, .. } => op.binds(),
            Self::AllOpen(terms) => terms.iter().any(|term| term.op.binds()),
            Self::AnyOpen(branches) => branches.iter().any(|(branch, _)| branch.binds()),
            Self::Search { sub, .. } => sub.binds(),
            Self::Fact { binds, .. } => !binds.is_empty(),
            Self::Pattern { vars, .. } => !vars.is_empty(),
            Self::All(items) | Self::Any(items) => items.iter().any(Self::binds),
            Self::Relation { sub, .. } => sub.binds(),
            Self::NthChild { of_rule, .. } => match of_rule {
                Some(OfRule::Frame(rule)) => rule.binds(),
                _ => false,
            },
            Self::Count { bind, .. } => bind.is_some(),
            Self::Call { returns, .. } => !returns.is_empty(),
            Self::Constrained { .. } | Self::NthCommit { .. } | Self::Recur { .. } => true,
            Self::EachRoot { .. } | Self::Excluded { .. } | Self::Gate(_) => false,
            Self::Host { .. } | Self::Every { .. } => false,
            Self::Seq(items) => items.iter().any(Self::binds),
            Self::Follow { sub, .. } => sub.binds(),
            Self::Parse { exports, .. } | Self::Exports { exports, .. } => !exports.is_empty(),
        }
    }
}

/// The candidate roots of a statement: nodes of `kinds` (every node without) in its files.
#[derive(Debug)]
pub(super) struct Statement {
    pub(super) slots: Vec<Slot>,
    pub(super) kinds: Option<Vec<u16>>,
    pub(super) op: Op,
    /// The recursive units the statement reads, by `Recur`'s index.
    pub(super) units: Vec<Unit>,
    /// The terminals' related-node walks the statement reads, by `Walk::Related`'s index.
    pub(super) sources: Vec<Source>,
}
