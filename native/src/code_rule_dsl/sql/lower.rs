//! Lowering from the compiled `AstMatcher` tree to the frame IR (`plan`). Term order,
//! `order_by_captures`, `Never` folding and grammar gaps stay the front end's: this walks what it
//! compiled. An operator the SQL engine does not cover yet fails with `Unsupported`; the engine
//! never answers for a clause it cannot lower.

mod leaves;
mod three;

use super::emit;
use super::plan::{
    CallReturn, CallSlot, MASK_BITS, Mask, NodeSet, OfRule, Op, Slot, SlotId, SlotKind, Source,
    Statement, Stop, Unit, Unsupported, Walk, mask,
};
use super::recursion::{self, Cycle, Recursion, UtilKey, util_key};
use super::terminals::{
    FACT_COLUMNS, FactTerm, Leaf, PATTERN_COLUMNS, PatternTerm, Terminal, Verdicts,
};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, CaptureAnalysis, GrammarMatcher, Related, Semantic, StopBy, UtilRef, reads_values,
};
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The scope of the slots no capture of the rule names: the root a row is evaluated for
/// (`violation.each`).
pub(super) const INTERNAL: usize = usize::MAX;

/// The internal slot holding, in a unit's loop, the demand row a row is evaluated for: what the
/// loop records each recursive reference's read from (`emit::Emitter::recur`).
pub(super) const ORIGIN: &str = "\0origin";

/// The internal slot holding, in the body of a unit keyed by its following context
/// (`plan::Unit::following`), the context its key carries.
const FOLLOWING: &str = "\0following";

/// While a unit of a following cycle (`recursion::Cycle::following`) is lowered: the cycle's
/// members, and the slot holding the following context where the lowering is, the unit's own
/// (`FOLLOWING`) or that of the guarded sub-rule it lies in (`Op::Follow`'s `out`).
#[derive(Clone)]
struct Following {
    keys: std::rc::Rc<HashSet<UtilKey>>,
    slot: SlotId,
}

/// A per-batch shared stage: a rule matched in a fresh environment at every node of its root kinds
/// in the files of its dialect, computed once per batch into `stage_<index>`. Stop rules and
/// context-free utilities are stages; equal ones (equal SQL) are one, keyed by digest. The last
/// of its statements selects the stage's distinct `(file, node)` rows; its root frame reads the
/// `/*FILES*/` the driver substitutes. A three-valued clause's stages are three-valued (`three`):
/// they keep the nodes their rule is unknown at, with `unk`.
pub(super) struct Stage {
    pub(super) dialect: GrammarDialect,
    pub(super) sql: emit::Emitted,
    pub(super) three: bool,
}

/// The terminals and stages a set of clauses shares.
#[derive(Default)]
pub(super) struct ProgramBuilder {
    pub(super) terminals: Vec<Terminal>,
    pub(super) stages: Vec<Stage>,
    stage_index: HashMap<String, usize>,
    /// Each rule lowered as a stage, by its address and dialect: a stage is context-free, so a
    /// rule reached again (a utility many terms or utilities share) is lowered once, not once per
    /// path to it.
    stage_rules: HashMap<(usize, String), usize>,
    regexes: HashMap<(String, String), usize>,
    fresh_patterns: HashMap<usize, usize>,
    /// Leaf fact terminals by the leaf's address and the captures their keys carry: lowering a
    /// stage again (to key it by its SQL) reuses them.
    leaves: HashMap<(usize, Vec<(String, SlotKind)>), usize>,
    recursion: Recursion,
    /// The members of the cycles whose units are being lowered.
    in_progress: HashSet<UtilKey>,
    /// When set, an operator the engine does not lower is recorded here and lowered as `Never`,
    /// so a census reads every refusal of a clause, not only its first.
    pub(super) census: Option<Vec<Unsupported>>,
}

impl ProgramBuilder {
    /// Lowers one clause's grammar matcher to its statement; under `violation.each` (`each`, the
    /// enumerated capture), the statement records each row's root and excludes the ranges the
    /// driver reports per root (`Op::EachRoot`, `Op::Excluded`).
    pub(super) fn clause_each(
        &mut self,
        grammar: &GrammarMatcher,
        each: Option<&str>,
    ) -> Result<Statement, Unsupported> {
        let kinds = grammar
            .anchors
            .root_kinds()
            .map(|kinds| kinds.iter().copied().collect());
        let mut lowerer = Lowerer::new(self, grammar.language);
        lowerer.three = grammar.three_valued;
        lowerer.shared = recursion::repeated(&[&grammar.matcher]);
        let root = each.map(|name| {
            lowerer.exclude = Some(name.to_owned());
            let root = lowerer.slot(INTERNAL, "\0root", SlotKind::Node);
            lowerer.root = Some(root);
            root
        });
        let (op, _) = lowerer.lower(&grammar.matcher, 0, &BTreeSet::new())?;
        let op = match root {
            Some(root) => Op::All(vec![Op::EachRoot { root }, op]),
            None => op,
        };
        let op = lowerer.unknowable(op);
        Ok(lowerer.finish(kinds, op))
    }
}

type Bound = BTreeSet<SlotId>;

/// A recursive unit's identity in a statement: its utility, the scope it runs in (`INTERNAL` for
/// a parameterized call's frame) and the capture `violation.each` excludes there.
type UnitKey = (UtilKey, usize, Option<String>);

struct Lowerer<'p> {
    program: &'p mut ProgramBuilder,
    dialect: GrammarDialect,
    slots: Vec<Slot>,
    index: HashMap<(usize, String, SlotKind), SlotId>,
    scopes: usize,
    /// Whether the statement is three-valued (`GrammarMatcher::three_valued`, which its stages
    /// share): its `all`s, `any`s and relations lower to `AllOpen`, `AnyOpen` and `Search`.
    three: bool,
    /// The withheld-mask bit of each capture (scope, name) an `all` may withhold.
    bits: HashMap<(usize, String), usize>,
    /// The stop-rule stages whose verdict may be unknown.
    open: Vec<usize>,
    units: Vec<Unit>,
    /// Each unit's identity, and for a call's unit the scope it runs in.
    unit_index: HashMap<UnitKey, (usize, usize)>,
    /// Each unit's utility.
    unit_utils: Vec<UtilRef>,
    /// The capture `violation.each` excludes here, by its name in the current scope; `None` where
    /// nothing is excluded (no `each`, or a negation, a count, a probe or a call it is not passed
    /// to).
    exclude: Option<String>,
    /// The slot of each row's root, under `violation.each`.
    root: Option<SlotId>,
    /// The terminals' related-node walks lowered so far (`Walk::Related`).
    sources: Vec<Source>,
    /// While lowering a parsed tree's rule (`parse`): where its host is. Its nodes are no
    /// file's, so no per-batch stage (computed over the batch's files) may stand for a rule.
    tree: Option<leaves::TreeContext>,
    /// The non-recursive utilities the rule being lowered reaches through two or more
    /// references (`recursion::repeated`): each is a unit, its body lowered once.
    shared: HashSet<UtilKey>,
    /// The root-kinds analysis of the relations' sub-rules, made on first use (`walk_kinds`).
    kinds: Option<RootKinds>,
    /// Where a unit of a following cycle is being lowered: its cycle and current context.
    following: Option<Following>,
    /// Each guarded binding sub-rule's number in the statement (`Op::Follow::rule`), by its
    /// address: one per matcher, as the matcher's `FOLLOWING` keys it.
    follow_rules: HashMap<usize, usize>,
}

fn unsupported<T>(operator: &str) -> Result<T, Unsupported> {
    Err(Unsupported(operator.to_owned()))
}

/// Whether a rule reads, binds or counts no capture: its verdict at a node is the node's alone.
fn capture_free(rule: &AstMatcher) -> bool {
    let mut analysis = CaptureAnalysis::default();
    analysis.mentions(rule).is_empty() && analysis.counts(rule).is_empty()
}

/// The kinds a node must have for a rule to match it: at a node of any other kind the rule is
/// false, and three-valued not unknown either (`None` where any kind may). A relation's walk reads
/// only its sub-rule's kinds (`Op::Relation::kinds`); a stage holds only its rule's.
///
/// `all` is the intersection of its terms' kinds and `any` the union; `not { not { R } }` is
/// `R`'s, and a utility or call its rule's. Three-valued, an `all` term a row can reach with a
/// capture it mentions withheld passes the row through untouched (`match_all`), so only the
/// others' kinds bound the `all`'s. A utility reached again while it is being analysed (a
/// recursive reference) may match any kind.
pub(super) struct RootKinds {
    three: bool,
    captures: CaptureAnalysis,
    /// Each utility's rule's kinds, by the rule's address.
    utilities: HashMap<usize, Option<BTreeSet<u16>>>,
    /// The utilities being analysed.
    visiting: HashSet<usize>,
}

impl RootKinds {
    pub(super) fn new(three: bool) -> Self {
        Self {
            three,
            captures: CaptureAnalysis::default(),
            utilities: HashMap::new(),
            visiting: HashSet::new(),
        }
    }

    pub(super) fn of(&mut self, rule: &AstMatcher) -> Option<BTreeSet<u16>> {
        match rule {
            AstMatcher::Pattern(pattern) => pattern
                .potential_kinds()
                .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
            AstMatcher::Kind(kind) => kind
                .potential_kinds()
                .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
            AstMatcher::Any(items) => {
                let mut kinds = BTreeSet::new();
                for item in items {
                    kinds.extend(self.of(item)?);
                }
                Some(kinds)
            }
            AstMatcher::All(items) => {
                let mut kinds: Option<BTreeSet<u16>> = None;
                // The captures a term may withhold from the terms after it (`lower::all_open`).
                let mut held = BTreeSet::new();
                for item in items {
                    if self.three {
                        let checked = !self.captures.mentions(item).is_disjoint(&held);
                        if checked
                            || (!item.binds_nothing() && reads_values(item, &mut BTreeSet::new()))
                        {
                            held.extend(self.captures.binds(item));
                        }
                        if checked {
                            continue;
                        }
                    }
                    if let Some(own) = self.of(item) {
                        kinds = Some(match kinds {
                            Some(kinds) => kinds.intersection(&own).copied().collect(),
                            None => own,
                        });
                    }
                }
                kinds
            }
            AstMatcher::Not(inner) => match &**inner {
                AstMatcher::Not(rule) => self.of(rule),
                _ => None,
            },
            AstMatcher::Constrained(core) => self.of(&core.rule),
            AstMatcher::Util(util) => util.with_target(|rule| self.utility(rule)).flatten(),
            AstMatcher::Call(call) => call.util.with_target(|rule| self.utility(rule)).flatten(),
            AstMatcher::Never => Some(BTreeSet::new()),
            _ => None,
        }
    }

    fn utility(&mut self, rule: &AstMatcher) -> Option<BTreeSet<u16>> {
        let key = std::ptr::from_ref(rule) as usize;
        if let Some(kinds) = self.utilities.get(&key) {
            return kinds.clone();
        }
        if !self.visiting.insert(key) {
            return None;
        }
        let kinds = self.of(rule);
        self.visiting.remove(&key);
        self.utilities.insert(key, kinds.clone());
        kinds
    }
}

/// A relation's sub-rule the frame algebra cannot express: an `nthChild` whose `ofRule` binds,
/// directly (or as a utility's body). The matcher commits `ofRule`'s bindings at the node before
/// it tests the position (`NthChild::index`), and a relation walk tests each candidate in the
/// walk's own environment, so a candidate whose position fails leaves its binding for the later
/// candidates: they are not independent, and no argmin over them reproduces that.
fn walk_leak(sub: &Op) -> Result<(), Unsupported> {
    match sub {
        Op::NthChild {
            of_rule: Some(OfRule::Frame(rule)),
            ..
        } if rule.binds() => unsupported("nthChild with a binding ofRule as a relation's rule"),
        _ => Ok(()),
    }
}

fn kind_id(kind: &ast_grep_core::matcher::KindMatcher) -> u16 {
    kind.potential_kinds()
        .and_then(|kinds| kinds.iter().next())
        .map_or(u16::MAX, |kind| kind as u16)
}

/// The units each unit's body refers to (`Op::Recur`), in `out`.
fn referred(op: &Op, out: &mut Vec<usize>) {
    match op {
        Op::Recur { unit, .. } => out.push(*unit),
        Op::All(items) | Op::Any(items) | Op::Seq(items) => {
            items.iter().for_each(|item| referred(item, out));
        }
        Op::Every { sub, .. }
        | Op::Parse { body: sub, .. }
        | Op::Host { rule: sub, .. }
        | Op::Follow { sub, .. } => {
            referred(sub, out);
        }
        Op::AllOpen(terms) => terms.iter().for_each(|term| referred(&term.op, out)),
        Op::AnyOpen(branches) => branches
            .iter()
            .for_each(|(branch, _)| referred(branch, out)),
        Op::Not(inner) | Op::Unknowable { op: inner, .. } => referred(inner, out),
        Op::Relation { sub, .. } | Op::Count { sub, .. } | Op::Search { sub, .. } => {
            referred(sub, out);
        }
        Op::NthChild {
            of_rule: Some(OfRule::Frame(rule)),
            ..
        } => referred(rule, out),
        Op::Call { body, .. } => referred(body, out),
        Op::Constrained { rule, constraints } => {
            referred(rule, out);
            constraints
                .iter()
                .for_each(|(_, constraint)| referred(constraint, out));
        }
        Op::NthCommit { nth, commit } => {
            referred(nth, out);
            referred(commit, out);
        }
        Op::Kinds(_)
        | Op::Regex(_)
        | Op::Pattern { .. }
        | Op::Never
        | Op::NthChild { .. }
        | Op::EachRoot { .. }
        | Op::Excluded { .. }
        | Op::Fact { .. }
        | Op::Gate(_)
        | Op::Exports { .. } => {}
    }
}

/// Numbers the strongly connected components of the units' reference graph (Tarjan): units that
/// reach each other share one loop.
fn assign_sccs(units: &mut [Unit]) {
    struct Tarjan<'u> {
        edges: &'u [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        next: usize,
        scc: Vec<usize>,
        sccs: usize,
    }
    impl Tarjan<'_> {
        fn visit(&mut self, unit: usize) {
            self.index[unit] = Some(self.next);
            self.low[unit] = self.next;
            self.next += 1;
            self.stack.push(unit);
            self.on_stack[unit] = true;
            for &to in &self.edges[unit] {
                match self.index[to] {
                    None => {
                        self.visit(to);
                        self.low[unit] = self.low[unit].min(self.low[to]);
                    }
                    Some(index) if self.on_stack[to] => {
                        self.low[unit] = self.low[unit].min(index);
                    }
                    Some(_) => {}
                }
            }
            if Some(self.low[unit]) == self.index[unit] {
                while let Some(member) = self.stack.pop() {
                    self.on_stack[member] = false;
                    self.scc[member] = self.sccs;
                    if member == unit {
                        break;
                    }
                }
                self.sccs += 1;
            }
        }
    }
    let edges: Vec<Vec<usize>> = units
        .iter()
        .map(|unit| {
            let mut out = Vec::new();
            referred(&unit.body, &mut out);
            out
        })
        .collect();
    let mut tarjan = Tarjan {
        edges: &edges,
        index: vec![None; units.len()],
        low: vec![0; units.len()],
        stack: Vec::new(),
        on_stack: vec![false; units.len()],
        next: 0,
        scc: vec![0; units.len()],
        sccs: 0,
    };
    for unit in 0..units.len() {
        if tarjan.index[unit].is_none() {
            tarjan.visit(unit);
        }
    }
    for (unit, scc) in units.iter_mut().zip(tarjan.scc) {
        unit.scc = scc;
    }
}

impl<'p> Lowerer<'p> {
    fn new(program: &'p mut ProgramBuilder, dialect: GrammarDialect) -> Self {
        Self {
            program,
            dialect,
            slots: Vec::new(),
            index: HashMap::new(),
            scopes: 0,
            three: false,
            bits: HashMap::new(),
            open: Vec::new(),
            units: Vec::new(),
            unit_index: HashMap::new(),
            unit_utils: Vec::new(),
            exclude: None,
            root: None,
            sources: Vec::new(),
            tree: None,
            shared: HashSet::new(),
            kinds: None,
            following: None,
            follow_rules: HashMap::new(),
        }
    }

    /// The kinds a walk's candidates must have for `rule` to hold, or be unknown, at them.
    fn walk_kinds(&mut self, rule: &AstMatcher) -> Option<Vec<u16>> {
        let three = self.three;
        self.kinds
            .get_or_insert_with(|| RootKinds::new(three))
            .of(rule)
            .map(|kinds| kinds.into_iter().collect())
    }

    /// The statement of what was lowered: its root kinds, its operator and its units.
    fn finish(self, kinds: Option<Vec<u16>>, op: Op) -> Statement {
        let mut units = self.units;
        assign_sccs(&mut units);
        Statement {
            slots: self.slots,
            kinds,
            op,
            units,
            sources: self.sources,
        }
    }

    /// `inner` with `exclude` as the capture `violation.each` excludes.
    fn with_exclude<T>(
        &mut self,
        exclude: Option<String>,
        inner: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved = std::mem::replace(&mut self.exclude, exclude);
        let result = inner(self);
        self.exclude = saved;
        result
    }

    /// `op` followed by `Excluded` where `violation.each` excludes here and the enumerated
    /// capture has a node slot in `scope`: where no slot exists, no row binds it here.
    fn excluding(&self, scope: usize, op: Op) -> Op {
        let (Some(name), Some(root)) = (&self.exclude, self.root) else {
            return op;
        };
        match self.existing(scope, name, SlotKind::Node) {
            Some(slot) => Op::All(vec![op, Op::Excluded { name: slot, root }]),
            None => op,
        }
    }

    /// Whether a rule lowered here, in a three-valued clause, may run on a binding of the capture
    /// `violation.each` excludes (its node slot may be bound on entry): its choice points then
    /// pass over every true candidate where that binding was reported, which a stage, matched
    /// once excluding nothing, cannot stand for (`emit/each.rs`).
    fn excluding_bound(&self, scope: usize, bound: &Bound) -> bool {
        self.three
            && self.root.is_some()
            && self
                .exclude
                .as_ref()
                .and_then(|name| self.existing(scope, name, SlotKind::Node))
                .is_some_and(|slot| bound.contains(&slot))
    }

    /// `walk_leak` for a relation's rule, also through a recursive unit: a relation in a context
    /// `violation.each` excludes in tests each candidate on a trial environment and leaks nothing.
    fn leaks(&self, sub: &Op) -> Result<(), Unsupported> {
        if self.exclude.is_some() {
            return Ok(());
        }
        walk_leak(sub)?;
        if let Op::Recur { unit, .. } = sub {
            let leaky = self.unit_utils[*unit]
                .with_target(|rule| match rule {
                    AstMatcher::NthChild(nth) => nth
                        .of_rule
                        .as_ref()
                        .is_some_and(|rule| !CaptureAnalysis::default().binds(rule).is_empty()),
                    _ => false,
                })
                .unwrap_or(false);
            if leaky {
                return unsupported("nthChild with a binding ofRule as a relation's rule");
            }
        }
        Ok(())
    }

    /// For a non-recursive utility the rule reaches through two or more references (`shared`):
    /// the one-member component its unit belongs to. Its body is then lowered once, as a unit
    /// (`unit`), whose loop finishes in one pass (no member reads another), and every reference
    /// demands its keys and reads its answers, as a recursive reference does, instead of lowering
    /// a copy of it. Not inside a parsed tree (units run over the batch's files), and not for an
    /// `nthChild` whose `ofRule` binds, which a unit cannot stand for under a relation (`leaks`).
    fn shared_unit(&self, util: &UtilRef) -> Option<Cycle> {
        let key = util_key(util);
        if self.tree.is_some() || !self.shared.contains(&key) {
            return None;
        }
        let leaky = util
            .with_target(|rule| match rule {
                AstMatcher::NthChild(nth) => nth
                    .of_rule
                    .as_ref()
                    .is_some_and(|rule| !CaptureAnalysis::default().binds(rule).is_empty()),
                _ => false,
            })
            .unwrap_or(true);
        (!leaky).then(|| Cycle {
            keys: vec![key],
            refused: None,
            following: false,
        })
    }

    /// The following context a reference to a unit of `cycle` passes: the current one, where the
    /// reference lies in a unit of that cycle and the cycle is keyed by its context; none
    /// elsewhere. A cycle's guarded sub-rules lie in its members' rules, so no reference from
    /// outside it holds a pair it can meet again: entering it, its context starts empty.
    fn context(&self, cycle: &Cycle) -> Option<SlotId> {
        self.following
            .as_ref()
            .filter(|following| cycle.following && following.keys.contains(&cycle.keys[0]))
            .map(|following| following.slot)
    }

    /// The number of a guarded binding sub-rule (`Op::Follow::rule`).
    fn follow_rule(&mut self, rule: &AstMatcher) -> usize {
        let next = self.follow_rules.len();
        *self
            .follow_rules
            .entry(std::ptr::from_ref(rule) as usize)
            .or_insert(next)
    }

    /// The unit of `util`, a member of `cycle` running in `scope` (a call's runs in a frame of its
    /// own, whose key is the `params` it may bind), lowered on first use: its key and the scope
    /// it runs in.
    fn unit(
        &mut self,
        cycle: &Cycle,
        util: &UtilRef,
        params: Option<&[String]>,
        scope: usize,
        exclude: Option<String>,
    ) -> Result<(usize, usize), Unsupported> {
        let identity = (
            util_key(util),
            if params.is_some() { INTERNAL } else { scope },
            exclude.clone(),
        );
        if let Some(&found) = self.unit_index.get(&identity) {
            return Ok(found);
        }
        let own = match params {
            Some(_) => {
                self.scopes += 1;
                self.scopes
            }
            None => scope,
        };
        let held = self.program.recursion.scope_kinds(util);
        let mut key: Vec<SlotId> = held
            .iter()
            .filter(|(name, _)| params.is_none_or(|params| params.contains(name)))
            .map(|(name, kind)| self.slot(own, name, *kind))
            .collect();
        // A utility sharing its caller's scope also reads captures it does not hold (a
        // `compare` against the caller's binding): its answer depends on them as well, so they
        // are its key too, every slot the scope has of them (as a terminal reads them,
        // `read_slots`), as the matcher's memo keys its verdicts by every capture it may read.
        // Mutant 18: the key drops the caller's captures the unit reads.
        if params.is_none() && !super::mutant(18) {
            let held_names: BTreeSet<&str> = held.iter().map(|(name, _)| name.as_str()).collect();
            let read: BTreeSet<String> = util
                .with_target(|rule| CaptureAnalysis::default().mentions(rule))
                .unwrap_or_default()
                .into_iter()
                .filter(|name| !held_names.contains(name.as_str()))
                .collect();
            key.extend(self.read_slots(own, &read).0);
        }
        if exclude.is_some()
            && let Some(root) = self.root
        {
            key.push(root);
        }
        // Three-valued, the body's verdicts read the excluded capture also where the utility
        // does not hold it (`excluding_bound`); the matcher's memo keys them by it too
        // (`memo::kept`).
        if self.three
            && let Some(name) = &exclude
        {
            let slot = self.slot(own, name, SlotKind::Node);
            if !key.contains(&slot) {
                key.push(slot);
            }
        }
        self.slot(INTERNAL, ORIGIN, SlotKind::Node);
        let following = cycle
            .following
            .then(|| self.slot(INTERNAL, FOLLOWING, SlotKind::Count));
        let id = self.units.len();
        self.units.push(Unit {
            key: key.clone(),
            body: Op::Never,
            scc: 0,
            following,
        });
        self.unit_utils.push(util.clone());
        self.unit_index.insert(identity, (id, own));
        let entered: Vec<UtilKey> = cycle
            .keys
            .iter()
            .copied()
            .filter(|key| self.program.in_progress.insert(*key))
            .collect();
        let bound: Bound = key.iter().copied().collect();
        // The body starts from the context its key carries; the lowering it interrupts (a unit
        // is lowered at its first reference) resumes with its own.
        let outer = std::mem::replace(
            &mut self.following,
            following.map(|slot| Following {
                keys: std::rc::Rc::new(cycle.keys.iter().copied().collect()),
                slot,
            }),
        );
        let lowered = self.with_exclude(exclude, |lowerer| {
            util.with_target(|rule| lowerer.lower(rule, own, &bound))
        });
        self.following = outer;
        for key in entered {
            self.program.in_progress.remove(&key);
        }
        if let Some(lowered) = lowered {
            self.units[id].body = lowered?.0;
        }
        Ok((id, own))
    }

    fn slot(&mut self, scope: usize, name: &str, kind: SlotKind) -> SlotId {
        if let Some(&slot) = self.index.get(&(scope, name.to_owned(), kind)) {
            return slot;
        }
        let id = self.slots.len();
        self.slots.push(Slot {
            name: name.to_owned(),
            kind,
            scope,
        });
        self.index.insert((scope, name.to_owned(), kind), id);
        id
    }

    fn existing(&self, scope: usize, name: &str, kind: SlotKind) -> Option<SlotId> {
        self.index.get(&(scope, name.to_owned(), kind)).copied()
    }

    /// What a rule's stage is keyed by besides the rule: its dialect, and whether it is
    /// three-valued (the same rule's stage differs between two- and three-valued clauses).
    fn stage_key(&self) -> String {
        if self.three {
            format!("{}:three-valued", self.dialect)
        } else {
            self.dialect.to_string()
        }
    }

    /// The statement's root: a three-valued one under `Op::Unknowable`, with as many mask
    /// columns as its withheld captures need and the stop stages that may be unknown.
    fn unknowable(&self, op: Op) -> Op {
        if !self.three {
            return op;
        }
        Op::Unknowable {
            op: Box::new(op),
            masks: self.bits.len().div_ceil(MASK_BITS),
            open: self.open.clone(),
        }
    }

    /// The withheld-mask bit of a capture an `all` may withhold.
    fn bit(&mut self, scope: usize, name: &str) -> usize {
        let next = self.bits.len();
        *self.bits.entry((scope, name.to_owned())).or_insert(next)
    }

    /// The mask of the names that have a bit (only those can be withheld).
    fn mask(&self, scope: usize, names: &BTreeSet<String>) -> Mask {
        let bits: Vec<usize> = names
            .iter()
            .filter_map(|name| self.bits.get(&(scope, name.clone())).copied())
            .collect();
        let words = bits.iter().max().map_or(0, |bit| bit / MASK_BITS + 1);
        mask(bits, words)
    }

    /// The one way a leaf the frame algebra does not express reaches a frame: an input-set
    /// terminal (`Op::Fact`) that evaluates `verdicts` once per distinct key of the frame. A key
    /// carries the bindings of `reads` (every slot of those names in `scope`) and, `at_node`, the
    /// node. The terminal answers true, false or unknown, and binds `binds` (in its column order)
    /// where it is true; it unifies them with the bindings it reads itself. `identity` names a
    /// terminal that evaluates alike wherever it is lowered with the same reads (one is shared).
    #[allow(clippy::too_many_arguments)]
    fn fact(
        &mut self,
        verdicts: impl FnOnce() -> Box<dyn Verdicts>,
        identity: Option<usize>,
        scope: usize,
        reads: &BTreeSet<String>,
        at_node: bool,
        binds: &[(String, SlotKind)],
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let mut bound_slots = Vec::new();
        for (name, kind) in binds {
            bound_slots.push(self.slot(scope, name, *kind));
        }
        self.fact_slots(
            verdicts,
            identity,
            scope,
            reads,
            at_node,
            &bound_slots,
            bound,
        )
    }

    /// `fact`, binding the slots `binds` (in the terminal's column order) whatever their scope:
    /// a leaf's internal slots (a binding's nodes, a parsed tree's number).
    #[allow(clippy::too_many_arguments)]
    fn fact_slots(
        &mut self,
        verdicts: impl FnOnce() -> Box<dyn Verdicts>,
        identity: Option<usize>,
        scope: usize,
        reads: &BTreeSet<String>,
        at_node: bool,
        binds: &[SlotId],
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let (slots, vars) = self.read_slots(scope, reads);
        let mut out = bound.clone();
        let mut width = 0;
        for &slot in binds {
            width += self.slots[slot].columns(slot).len();
            out.insert(slot);
        }
        if width > FACT_COLUMNS {
            return unsupported("terminal binding more than 16 columns");
        }
        let key = identity
            .filter(|_| binds.is_empty())
            .map(|identity| (identity, vars.clone()));
        let term = match key.as_ref().and_then(|key| self.program.leaves.get(key)) {
            Some(&term) => term,
            None => {
                let term = self.program.terminals.len();
                self.program.terminals.push(Terminal::Fact(FactTerm {
                    verdicts: verdicts(),
                    reads: vars,
                }));
                if let Some(key) = key {
                    self.program.leaves.insert(key, term);
                }
                term
            }
        };
        Ok((
            Op::Fact {
                term,
                reads: slots,
                at_node,
                binds: binds.to_vec(),
            },
            out,
        ))
    }

    /// The slots of `names` in `scope` (every kind each has), with their names and kinds in
    /// the same order: what a terminal's key carries of them.
    fn read_slots(
        &self,
        scope: usize,
        names: &BTreeSet<String>,
    ) -> (Vec<SlotId>, Vec<(String, SlotKind)>) {
        let mut slots = Vec::new();
        let mut vars = Vec::new();
        for name in names {
            for kind in [
                SlotKind::Node,
                SlotKind::List,
                SlotKind::Count,
                SlotKind::Text,
            ] {
                if let Some(slot) = self.existing(scope, name, kind) {
                    slots.push(slot);
                    vars.push((name.clone(), kind));
                }
            }
        }
        (slots, vars)
    }

    /// A leaf the matcher evaluates itself (`value`, `compare`, `layout`, `testUnknown`) as a
    /// fact terminal reading `reads`.
    fn leaf(
        &mut self,
        leaf: &AstMatcher,
        scope: usize,
        reads: &BTreeSet<String>,
        at_node: bool,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        self.fact(
            || Box::new(Leaf(leaf.clone())),
            Some(std::ptr::from_ref(leaf) as usize),
            scope,
            reads,
            at_node,
            &[],
            bound,
        )
    }

    /// The per-batch set of the nodes `rule` matches in a fresh environment: a stage lowered,
    /// emitted and keyed by its SQL. A capture-free `not R` that cannot be unknown is `R`'s set
    /// complemented: a negation holds at most nodes of the batch, `R` at few. Not one that
    /// reaches back into the following cycle being lowered: the matcher matches it with the
    /// binding sub-rules in progress, which a per-batch stage does not know.
    fn stage(&mut self, rule: &AstMatcher) -> Result<NodeSet, Unsupported> {
        if let Some(following) = &self.following
            && recursion::reaches(rule, &following.keys)
        {
            return unsupported("per-batch stage reaching back into a following recursion");
        }
        if let AstMatcher::Not(inner) = rule
            && capture_free(inner)
            && !(self.three && reads_values(inner, &mut BTreeSet::new()))
        {
            return Ok(self.stage(inner)?.complemented());
        }
        let address = (std::ptr::from_ref(rule) as usize, self.stage_key());
        let stage = match self.program.stage_rules.get(&address) {
            Some(&stage) => stage,
            None => {
                let stage = self.stage_of(rule)?;
                self.program.stage_rules.insert(address, stage);
                stage
            }
        };
        Ok(NodeSet {
            stage,
            complement: false,
        })
    }

    fn stage_of(&mut self, rule: &AstMatcher) -> Result<usize, Unsupported> {
        let kinds = RootKinds::new(self.three)
            .of(rule)
            .map(|kinds| kinds.into_iter().collect());
        let statement = {
            let mut lowerer = Lowerer::new(self.program, self.dialect);
            lowerer.three = self.three;
            lowerer.shared = recursion::repeated(&[rule]);
            let (op, _) = lowerer.lower(rule, 0, &BTreeSet::new())?;
            let op = lowerer.unknowable(op);
            lowerer.finish(kinds, op)
        };
        let sql = emit::stage(&statement);
        let digest = format!("{}:{}", self.dialect, sql.statements.join(";\n"));
        if let Some(&existing) = self.program.stage_index.get(&digest) {
            return Ok(existing);
        }
        let id = self.program.stages.len();
        self.program.stage_index.insert(digest, id);
        self.program.stages.push(Stage {
            dialect: self.dialect,
            sql,
            three: self.three,
        });
        Ok(id)
    }

    fn stop(&mut self, stop: &StopBy) -> Result<Stop, Unsupported> {
        Ok(match stop {
            StopBy::Neighbor => Stop::Neighbor,
            StopBy::End => Stop::End,
            StopBy::Rule(rule) => {
                // A stage holds the batch's files' nodes, not a parsed tree's.
                if self.tree.is_some() {
                    return unsupported("stopBy rule inside parse");
                }
                let set = self.stage(rule)?;
                // A stop rule unknown at a node ends the walk there and leaves it open (never a
                // complement: that rule cannot be unknown).
                if self.three
                    && reads_values(rule, &mut BTreeSet::new())
                    && !self.open.contains(&set.stage)
                {
                    self.open.push(set.stage);
                }
                Stop::Rule(set)
            }
        })
    }

    /// A relation's walk and its rule, from `inside`/`has`/`precedes`/`follows`.
    fn relation<'m>(
        &mut self,
        matcher: &'m AstMatcher,
    ) -> Result<Option<(Walk, &'m Related)>, Unsupported> {
        Ok(Some(match matcher {
            AstMatcher::Inside(related, stop) => (Walk::Inside(self.stop(stop)?), &**related),
            AstMatcher::Has(related, stop) => (Walk::Has(self.stop(stop)?), &**related),
            AstMatcher::Precedes(related, stop) => (Walk::Precedes(self.stop(stop)?), &**related),
            AstMatcher::Follows(related, stop) => (Walk::Follows(self.stop(stop)?), &**related),
            _ => return Ok(None),
        }))
    }

    fn lower(
        &mut self,
        matcher: &AstMatcher,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        match self.lower_term(matcher, scope, bound) {
            Err(refusal) if self.program.census.is_some() => {
                self.program
                    .census
                    .as_mut()
                    .expect("a census")
                    .push(refusal);
                Ok((Op::Never, bound.clone()))
            }
            lowered => lowered,
        }
    }

    fn lower_term(
        &mut self,
        matcher: &AstMatcher,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        match matcher {
            AstMatcher::Pattern(_) if self.tree.is_some() => unsupported("pattern inside parse"),
            AstMatcher::Pattern(pattern) => {
                let mut term = PatternTerm::new(pattern.clone(), self.dialect);
                if term.width() > PATTERN_COLUMNS {
                    return unsupported("pattern with more than 1024 capture columns");
                }
                let vars: Vec<SlotId> = term
                    .vars
                    .iter()
                    .map(|(name, kind)| self.slot(scope, name, *kind))
                    .collect();
                term.reads = (0..vars.len())
                    .filter(|&index| bound.contains(&vars[index]))
                    .collect();
                let reads: Vec<SlotId> = term.reads.iter().map(|&index| vars[index]).collect();
                let env = !reads.is_empty();
                let key = std::ptr::from_ref(pattern) as usize;
                let id = match (env, self.program.fresh_patterns.get(&key)) {
                    (false, Some(&id)) => id,
                    _ => {
                        let id = self.program.terminals.len();
                        self.program.terminals.push(Terminal::Pattern(term));
                        if !env {
                            self.program.fresh_patterns.insert(key, id);
                        }
                        id
                    }
                };
                let mut out = bound.clone();
                out.extend(vars.iter().copied());
                Ok((
                    Op::Pattern {
                        term: id,
                        env,
                        vars,
                        reads,
                    },
                    out,
                ))
            }
            AstMatcher::Kind(kind) => Ok((Op::Kinds(vec![kind_id(kind)]), bound.clone())),
            AstMatcher::Regex(regex) => {
                let key = (self.dialect.to_string(), regex.as_str().to_owned());
                let id = match self.program.regexes.get(&key) {
                    Some(&id) => id,
                    None => {
                        let id = self.program.terminals.len();
                        self.program.terminals.push(Terminal::Regex(regex.clone()));
                        self.program.regexes.insert(key, id);
                        id
                    }
                };
                Ok((Op::Regex(id), bound.clone()))
            }
            AstMatcher::All(items) if self.three => self.all_open(items, scope, bound),
            // A kind-only `any` is a kind test, but where `violation.each` may exclude the
            // binding it runs on, each of its true branches is passed over (`emit/each.rs`).
            AstMatcher::Any(items)
                if self.three
                    && (!items.iter().all(|item| matches!(item, AstMatcher::Kind(_)))
                        || self.excluding_bound(scope, bound)) =>
            {
                self.any_open(items, scope, bound)
            }
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Precedes(..)
            | AstMatcher::Follows(..)
            | AstMatcher::Field(_)
                if self.three =>
            {
                self.search(matcher, scope, bound)
            }
            AstMatcher::All(items) => {
                let mut ops = Vec::with_capacity(items.len());
                let mut current = bound.clone();
                for item in items {
                    let (op, out) = self.lower(item, scope, &current)?;
                    ops.push(op);
                    current = out;
                }
                Ok((Op::All(ops), current))
            }
            AstMatcher::Any(items) => {
                if items.iter().all(|item| matches!(item, AstMatcher::Kind(_))) {
                    let kinds = items
                        .iter()
                        .filter_map(|item| match item {
                            AstMatcher::Kind(kind) => Some(kind_id(kind)),
                            _ => None,
                        })
                        .collect();
                    return Ok((Op::Kinds(kinds), bound.clone()));
                }
                let mut ops = Vec::with_capacity(items.len());
                let mut out = bound.clone();
                for item in items {
                    let (op, branch) = self.lower(item, scope, bound)?;
                    ops.push(self.excluding(scope, op));
                    out.extend(branch);
                }
                Ok((Op::Any(ops), out))
            }
            AstMatcher::Not(inner) => {
                // Mutant 3: the probe ignores the captures bound before it.
                let fresh = Bound::new();
                let probe = if super::mutant(3) { &fresh } else { bound };
                let (op, _) =
                    self.with_exclude(None, |lowerer| lowerer.lower(inner, scope, probe))?;
                Ok((Op::Not(Box::new(op)), bound.clone()))
            }
            AstMatcher::Never => Ok((Op::Never, bound.clone())),
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Precedes(..)
            | AstMatcher::Follows(..) => {
                let (walk, related) = self.relation(matcher)?.expect("a relation");
                let (sub, out) = self.lower(related.rule(), scope, bound)?;
                self.leaks(&sub)?;
                let sub = self.excluding(scope, sub);
                let binds = sub.binds();
                Ok((
                    Op::Relation {
                        walk,
                        sub: Box::new(sub),
                        binds,
                        kinds: self.walk_kinds(related.rule()),
                    },
                    out,
                ))
            }
            AstMatcher::Field(relation) => {
                let stop = self.stop(&relation.stop)?;
                let walk = if relation.inside {
                    Walk::FieldInside(relation.field, stop)
                } else {
                    Walk::FieldHas(relation.field, stop)
                };
                let (sub, out) = self.lower(&relation.rule, scope, bound)?;
                self.leaks(&sub)?;
                let sub = self.excluding(scope, sub);
                let binds = sub.binds();
                Ok((
                    Op::Relation {
                        walk,
                        sub: Box::new(sub),
                        binds,
                        kinds: self.walk_kinds(&relation.rule),
                    },
                    out,
                ))
            }
            AstMatcher::NthChild(nth) => {
                let (of_rule, out) = match &nth.of_rule {
                    None => (None, bound.clone()),
                    Some(rule)
                        if capture_free(rule)
                            && self.tree.is_none()
                            && !recursion::reaches(rule, &self.program.in_progress) =>
                    {
                        (Some(OfRule::Stage(self.stage(rule)?)), bound.clone())
                    }
                    Some(rule) => {
                        // The siblings are probed excluding nothing.
                        let (op, out) =
                            self.with_exclude(None, |lowerer| lowerer.lower(rule, scope, bound))?;
                        (Some(OfRule::Frame(Box::new(op))), out)
                    }
                };
                let committed = match (&nth.of_rule, &of_rule) {
                    (Some(rule), Some(OfRule::Frame(probe)))
                        if self.exclude.is_some() && probe.binds() =>
                    {
                        // `NthChild::index` re-matches `ofRule` at the node excluding, and drops
                        // its verdict: an unknown re-match leaves `UNKNOWN` set on a true or a
                        // false `nthChild` (see `emit/each.rs`), which no frame verdict is.
                        if self.three && reads_values(rule, &mut BTreeSet::new()) {
                            return unsupported(
                                "three-valued nthChild whose excluding commit may be unknown",
                            );
                        }
                        Some(self.lower(rule, scope, bound)?.0)
                    }
                    _ => None,
                };
                let nth = Op::NthChild {
                    step: nth.position.step,
                    offset: nth.position.offset,
                    reverse: nth.reverse,
                    of_rule,
                };
                Ok((
                    match committed {
                        Some(commit) => Op::NthCommit {
                            nth: Box::new(nth),
                            commit: Box::new(commit),
                        },
                        None => nth,
                    },
                    out,
                ))
            }
            AstMatcher::Count(count) => {
                let Some((walk, related)) = self.relation(&count.relation)? else {
                    // A count over any other term counts nothing and holds nowhere.
                    return Ok((Op::Never, bound.clone()));
                };
                let (sub, _) =
                    self.with_exclude(None, |lowerer| lowerer.lower(related.rule(), scope, bound))?;
                let mut out = bound.clone();
                let bind = count.binding.as_ref().map(|binding| {
                    let slot = self.slot(scope, &binding.name, SlotKind::Count);
                    out.insert(slot);
                    (slot, binding.offset)
                });
                Ok((
                    Op::Count {
                        walk,
                        sub: Box::new(sub),
                        min: count.min,
                        max: count.max,
                        bind,
                        kinds: self.walk_kinds(related.rule()),
                    },
                    out,
                ))
            }
            AstMatcher::Util(util) => {
                // A utility whose rules are gone or not compiled yet matches nothing, as natively.
                if util.with_target(|_| ()).is_none() {
                    return Ok((Op::Never, bound.clone()));
                }
                if let Some(cycle) = self.program.recursion.cycle(util) {
                    recursion::refusal(&cycle)?;
                    if self.tree.is_some() {
                        return unsupported("recursive utility inside parse");
                    }
                    let exclude = self.exclude.clone();
                    let (unit, _) = self.unit(&cycle, util, None, scope, exclude)?;
                    let mut out = bound.clone();
                    out.extend(self.units[unit].key.iter().copied());
                    let context = self.context(&cycle);
                    return Ok((
                        Op::Recur {
                            unit,
                            slots: None,
                            context,
                        },
                        out,
                    ));
                }
                // A `matches` evaluates its utility on the frame's nodes only (demand-driven, as
                // the matcher's memo): inlined, or a unit where two or more references reach it.
                // Only whole-set readers (stop rules, `ofRule`) make a capture-free rule a
                // per-batch stage over every node of its root kinds.
                if let Some(cycle) = self.shared_unit(util) {
                    let exclude = self.exclude.clone();
                    let (unit, _) = self.unit(&cycle, util, None, scope, exclude)?;
                    let mut out = bound.clone();
                    out.extend(self.units[unit].key.iter().copied());
                    return Ok((
                        Op::Recur {
                            unit,
                            slots: None,
                            context: None,
                        },
                        out,
                    ));
                }
                util.with_target(|rule| self.lower(rule, scope, bound))
                    .expect("a filled utility")
            }
            AstMatcher::Call(call) => {
                if call.util.with_target(|_| ()).is_none() {
                    return Ok((Op::Never, bound.clone()));
                }
                // The enumerated capture is excluded in the frame under the first parameter it
                // is passed as; a frame it is not passed to excludes nothing (`each::framed`).
                let exclude = self.exclude.as_ref().and_then(|name| {
                    call.args
                        .iter()
                        .find(|(_, caller)| caller == name)
                        .map(|(param, _)| param.clone())
                });
                self.scopes += 1;
                let callee = self.scopes;
                let kinds = [
                    SlotKind::Node,
                    SlotKind::List,
                    SlotKind::Count,
                    SlotKind::Text,
                ];
                let cycle = match self.program.recursion.cycle(&call.util) {
                    Some(cycle) => {
                        recursion::refusal(&cycle)?;
                        Some(cycle)
                    }
                    None => self.shared_unit(&call.util).map(std::rc::Rc::new),
                };
                let (body, returned): (Op, Option<Bound>) = match cycle {
                    Some(cycle) => {
                        let params: Vec<String> =
                            call.args.iter().map(|(param, _)| param.clone()).collect();
                        let (unit, _) =
                            self.unit(&cycle, &call.util, Some(&params), scope, exclude)?;
                        // The call's own copy of the frame's key: the unit's frame is the
                        // same for every call, and a call inside the unit's own body must
                        // not overwrite the parameters of the frame it runs in.
                        let key = self.units[unit].key.clone();
                        let slots = key
                            .iter()
                            .map(|&slot| {
                                if Some(slot) == self.root {
                                    return slot;
                                }
                                let (name, kind) =
                                    (self.slots[slot].name.clone(), self.slots[slot].kind);
                                self.slot(callee, &name, kind)
                            })
                            .collect();
                        (
                            Op::Recur {
                                unit,
                                slots: Some(slots),
                                context: self.context(&cycle),
                            },
                            None,
                        )
                    }
                    None => {
                        // The parameters the caller's bindings may fill on entry.
                        let mut entry_bound = Bound::new();
                        for (param, caller) in &call.args {
                            for kind in kinds {
                                if let Some(slot) = self.existing(scope, caller, kind)
                                    && bound.contains(&slot)
                                {
                                    entry_bound.insert(self.slot(callee, param, kind));
                                }
                            }
                        }
                        let (body, body_out) = self.with_exclude(exclude, |lowerer| {
                            call.util
                                .with_target(|rule| lowerer.lower(rule, callee, &entry_bound))
                                .expect("a filled utility")
                        })?;
                        (body, Some(body_out))
                    }
                };
                let mut entry = Vec::new();
                for (param, caller) in &call.args {
                    for kind in kinds {
                        if let (Some(callee_slot), Some(caller_slot)) = (
                            self.existing(callee, param, kind),
                            self.existing(scope, caller, kind),
                        ) {
                            entry.push(CallSlot {
                                kind,
                                callee: callee_slot,
                                caller: caller_slot,
                            });
                        }
                    }
                }
                let mut returns = Vec::new();
                let mut out = bound.clone();
                let mut callers: Vec<&str> = Vec::new();
                for (_, caller) in &call.args {
                    if !callers.contains(&caller.as_str()) {
                        callers.push(caller);
                    }
                }
                for caller in callers {
                    for kind in kinds {
                        let params: Vec<SlotId> = call
                            .args
                            .iter()
                            .filter(|(_, mapped)| mapped == caller)
                            .filter_map(|(param, _)| self.existing(callee, param, kind))
                            .filter(|slot| returned.as_ref().is_none_or(|out| out.contains(slot)))
                            .collect();
                        if params.is_empty() {
                            continue;
                        }
                        let caller_slot = self.slot(scope, caller, kind);
                        out.insert(caller_slot);
                        returns.push(CallReturn {
                            kind,
                            caller: caller_slot,
                            params,
                        });
                    }
                }
                Ok((
                    Op::Call {
                        entry,
                        body: Box::new(body),
                        returns,
                    },
                    out,
                ))
            }
            AstMatcher::Constrained(core) => {
                let (rule, mut current) = self.lower(&core.rule, scope, bound)?;
                let mut constraints = Vec::new();
                // Mutant 14: the constraints in reverse order.
                let (flip, count) = (super::mutant(14), core.constraints.len());
                for index in 0..count {
                    let (name, constraint) =
                        &core.constraints[if flip { count - 1 - index } else { index }];
                    let Some(slot) = self.existing(scope, name, SlotKind::Node) else {
                        // Never bound, so never checked.
                        continue;
                    };
                    let (op, out) = self.lower(constraint, scope, &current)?;
                    constraints.push((slot, op));
                    current = out;
                }
                Ok((
                    Op::Constrained {
                        rule: Box::new(rule),
                        constraints,
                    },
                    current,
                ))
            }
            // `compare` reads the bindings of its sides (or of a transform variable's roots) and
            // never the node.
            AstMatcher::Compare(_) => {
                let reads = CaptureAnalysis::default().reads(matcher);
                self.leaf(matcher, scope, &reads, false, bound)
            }
            AstMatcher::Semantic(Semantic::Value(_)) => {
                self.leaf(matcher, scope, &BTreeSet::new(), true, bound)
            }
            AstMatcher::Semantic(Semantic::Layout(layout)) => {
                let reads = layout.reads().map(str::to_owned).collect();
                self.leaf(matcher, scope, &reads, true, bound)
            }
            AstMatcher::Semantic(Semantic::Resolves(_)) => {
                self.leaf(matcher, scope, &BTreeSet::new(), true, bound)
            }
            AstMatcher::Semantic(semantic) => self.semantic(matcher, semantic, scope, bound),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => self.leaf(matcher, scope, &BTreeSet::new(), true, bound),
        }
    }
}
