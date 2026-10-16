//! Lowering from the compiled `AstMatcher` tree to the frame IR (`plan`). Term order,
//! `order_by_captures`, `Never` folding and grammar gaps stay the front end's: this walks what it
//! compiled. An operator the SQL engine does not cover yet fails with `Unsupported`; the engine
//! never answers for a clause it cannot lower.

use super::emit;
use super::plan::{
    CallReturn, CallSlot, OfRule, Op, Slot, SlotId, SlotKind, Statement, Stop, Unit, Unsupported,
    Walk,
};
use super::recursion::{self, Cycle, Recursion, UtilKey, util_key};
use super::terminals::{PATTERN_COLUMNS, PatternTerm, Terminal};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, CaptureAnalysis, GrammarMatcher, Related, Semantic, StopBy, UtilRef,
};
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The scope of the slots no capture of the rule names: the root a row is evaluated for
/// (`violation.each`).
pub(super) const INTERNAL: usize = usize::MAX;

/// The internal slot holding, in a unit's loop, the demand row a row is evaluated for: what the
/// loop records each recursive reference's read from (`emit::Emitter::recur`).
pub(super) const ORIGIN: &str = "\0origin";

/// A per-batch shared stage: a rule matched in a fresh environment at every node of its root kinds
/// in the files of its dialect, computed once per batch into `stage_<index>`. Stop rules and
/// context-free utilities are stages; equal ones (equal SQL) are one, keyed by digest. The last
/// of `statements` selects the stage's distinct `(file, node)` rows; its root frame reads the
/// `/*FILES*/` the driver substitutes.
pub(super) struct Stage {
    pub(super) dialect: GrammarDialect,
    pub(super) statements: Vec<String>,
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
    recursion: Recursion,
    /// A capture-free recursive utility's stage, per utility and dialect.
    recursive_stages: HashMap<(UtilKey, String), usize>,
    /// The stages of each capture-free cycle's members, by the digest of the cycle's loop.
    recursive_stage_index: HashMap<String, Vec<usize>>,
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
        if grammar.three_valued {
            let refusal = Unsupported("three-valued clause".into());
            match &mut self.census {
                Some(census) => census.push(refusal),
                None => return Err(refusal),
            }
        }
        let kinds = grammar
            .anchors
            .root_kinds()
            .map(|kinds| kinds.iter().copied().collect());
        let mut lowerer = Lowerer::new(self, grammar.language);
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
    /// While lowering a capture-free cycle's stage: its members' units.
    stage_members: HashMap<UtilKey, usize>,
}

fn unsupported<T>(operator: &str) -> Result<T, Unsupported> {
    Err(Unsupported(operator.to_owned()))
}

/// Whether a rule reads, binds or counts no capture: its verdict at a node is the node's alone.
fn capture_free(rule: &AstMatcher) -> bool {
    let mut analysis = CaptureAnalysis::default();
    analysis.mentions(rule).is_empty() && analysis.counts(rule).is_empty()
}

/// The kinds a node must have for `rule` to match it; `None` where any may.
fn root_kinds(rule: &AstMatcher, depth: usize) -> Option<BTreeSet<u16>> {
    if depth > 32 {
        return None;
    }
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
                kinds.extend(root_kinds(item, depth + 1)?);
            }
            Some(kinds)
        }
        AstMatcher::All(items) => items
            .iter()
            .filter_map(|item| root_kinds(item, depth + 1))
            .reduce(|a, b| a.intersection(&b).copied().collect()),
        AstMatcher::Constrained(core) => root_kinds(&core.rule, depth + 1),
        AstMatcher::Util(util) => util
            .with_target(|rule| root_kinds(rule, depth + 1))
            .flatten(),
        AstMatcher::Never => Some(BTreeSet::new()),
        _ => None,
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
        Op::All(items) | Op::Any(items) => items.iter().for_each(|item| referred(item, out)),
        Op::Not(inner) => referred(inner, out),
        Op::Relation { sub, .. } | Op::Count { sub, .. } => referred(sub, out),
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
        | Op::Stage(_)
        | Op::EachRoot { .. }
        | Op::Excluded { .. } => {}
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
            units: Vec::new(),
            unit_index: HashMap::new(),
            unit_utils: Vec::new(),
            exclude: None,
            root: None,
            stage_members: HashMap::new(),
        }
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
        if exclude.is_some()
            && let Some(root) = self.root
        {
            key.push(root);
        }
        self.slot(INTERNAL, ORIGIN, SlotKind::Node);
        let id = self.units.len();
        self.units.push(Unit {
            key: key.clone(),
            body: Op::Never,
            seed: None,
            scc: 0,
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
        let lowered = self.with_exclude(exclude, |lowerer| {
            util.with_target(|rule| lowerer.lower(rule, own, &bound))
        });
        for key in entered {
            self.program.in_progress.remove(&key);
        }
        if let Some(lowered) = lowered {
            self.units[id].body = lowered?.0;
        }
        Ok((id, own))
    }

    /// The stage of `util`, a member of the capture-free `cycle`: every member is a unit of one
    /// statement, evaluated over every node of its root kinds in a loop, and each member's
    /// matches are a stage of their own. Equal cycles (equal SQL) are one.
    fn recursive_stage(&mut self, cycle: &Cycle, util: &UtilRef) -> Result<usize, Unsupported> {
        let dialect = self.dialect.to_string();
        if let Some(&stage) = self
            .program
            .recursive_stages
            .get(&(util_key(util), dialect.clone()))
        {
            return Ok(stage);
        }
        let mut lowerer = Lowerer::new(self.program, self.dialect);
        lowerer.slot(INTERNAL, ORIGIN, SlotKind::Node);
        for (index, member) in cycle.members.iter().enumerate() {
            let seed = member
                .with_target(|rule| root_kinds(rule, 0).map(|kinds| kinds.into_iter().collect()))
                .flatten();
            lowerer.units.push(Unit {
                key: Vec::new(),
                body: Op::Never,
                seed: Some(seed),
                scc: 0,
            });
            lowerer.unit_utils.push(member.clone());
            lowerer.stage_members.insert(cycle.keys[index], index);
        }
        let entered: Vec<UtilKey> = cycle
            .keys
            .iter()
            .copied()
            .filter(|key| lowerer.program.in_progress.insert(*key))
            .collect();
        let mut bodies = Vec::new();
        for member in &cycle.members {
            let lowered = member.with_target(|rule| lowerer.lower(rule, 0, &BTreeSet::new()));
            match lowered {
                Some(Ok((body, _))) => bodies.push(body),
                Some(Err(error)) => {
                    for key in &entered {
                        lowerer.program.in_progress.remove(key);
                    }
                    return Err(error);
                }
                None => bodies.push(Op::Never),
            }
        }
        for key in &entered {
            lowerer.program.in_progress.remove(key);
        }
        for (unit, body) in lowerer.units.iter_mut().zip(bodies) {
            unit.body = body;
        }
        let statement = lowerer.finish(None, Op::Never);
        let leader = emit::recursive_stage(&statement);
        let digest = format!("{dialect}:recursive:{}", leader.join(";\n"));
        let stages = match self.program.recursive_stage_index.get(&digest) {
            Some(stages) => stages.clone(),
            None => {
                let first = self.program.stages.len();
                let namespace = emit::stage_namespace(first);
                self.program.stages.push(Stage {
                    dialect: self.dialect,
                    statements: leader,
                });
                let mut stages = vec![first];
                for unit in 1..cycle.members.len() {
                    stages.push(self.program.stages.len());
                    self.program.stages.push(Stage {
                        dialect: self.dialect,
                        statements: vec![emit::recursive_member(&namespace, unit)],
                    });
                }
                self.program
                    .recursive_stage_index
                    .insert(digest, stages.clone());
                stages
            }
        };
        for (key, stage) in cycle.keys.iter().zip(&stages) {
            self.program
                .recursive_stages
                .insert((*key, dialect.clone()), *stage);
        }
        let index = cycle
            .keys
            .iter()
            .position(|key| *key == util_key(util))
            .expect("a member");
        Ok(stages[index])
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

    /// A stage for `rule`, matched in a fresh environment: lowered, emitted and keyed by its SQL.
    fn stage(&mut self, rule: &AstMatcher) -> Result<usize, Unsupported> {
        let address = (std::ptr::from_ref(rule) as usize, self.dialect.to_string());
        if let Some(&stage) = self.program.stage_rules.get(&address) {
            return Ok(stage);
        }
        let stage = self.stage_of(rule)?;
        self.program.stage_rules.insert(address, stage);
        Ok(stage)
    }

    fn stage_of(&mut self, rule: &AstMatcher) -> Result<usize, Unsupported> {
        let kinds = root_kinds(rule, 0).map(|kinds| kinds.into_iter().collect());
        let statement = {
            let mut lowerer = Lowerer::new(self.program, self.dialect);
            let (op, _) = lowerer.lower(rule, 0, &BTreeSet::new())?;
            lowerer.finish(kinds, op)
        };
        let statements = emit::stage(&statement);
        let digest = format!("{}:{}", self.dialect, statements.join(";\n"));
        if let Some(&existing) = self.program.stage_index.get(&digest) {
            return Ok(existing);
        }
        let id = self.program.stages.len();
        self.program.stage_index.insert(digest, id);
        self.program.stages.push(Stage {
            dialect: self.dialect,
            statements,
        });
        Ok(id)
    }

    fn stop(&mut self, stop: &StopBy) -> Result<Stop, Unsupported> {
        Ok(match stop {
            StopBy::Neighbor => Stop::Neighbor,
            StopBy::End => Stop::End,
            StopBy::Rule(rule) => Stop::Rule(self.stage(rule)?),
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
                let (op, _) = self.with_exclude(None, |lowerer| lowerer.lower(inner, scope, bound))?;
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
                    },
                    out,
                ))
            }
            AstMatcher::NthChild(nth) => {
                let (of_rule, out) = match &nth.of_rule {
                    None => (None, bound.clone()),
                    Some(rule)
                        if capture_free(rule)
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
                let (sub, _) = self.with_exclude(None, |lowerer| {
                    lowerer.lower(related.rule(), scope, bound)
                })?;
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
                    if let Some(&unit) = self.stage_members.get(&util_key(util)) {
                        return Ok((Op::Recur { unit, slots: None }, bound.clone()));
                    }
                    if cycle.capture_free {
                        let stage = self.recursive_stage(&cycle, util)?;
                        return Ok((Op::Stage(stage), bound.clone()));
                    }
                    let exclude = self.exclude.clone();
                    let (unit, _) = self.unit(&cycle, util, None, scope, exclude)?;
                    let mut out = bound.clone();
                    out.extend(self.units[unit].key.iter().copied());
                    return Ok((Op::Recur { unit, slots: None }, out));
                }
                util.with_target(|rule| {
                    if capture_free(rule) {
                        self.stage(rule)
                            .map(|stage| (Op::Stage(stage), bound.clone()))
                    } else {
                        self.lower(rule, scope, bound)
                    }
                })
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
                let kinds = [SlotKind::Node, SlotKind::List, SlotKind::Count];
                let (body, returned): (Op, Option<Bound>) =
                    match self.program.recursion.cycle(&call.util) {
                        Some(cycle) => {
                            recursion::refusal(&cycle)?;
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
                for (name, constraint) in &core.constraints {
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
            AstMatcher::Compare(_) => unsupported("compare"),
            AstMatcher::Semantic(semantic) => unsupported(match semantic {
                Semantic::Resolves(_) => "resolves",
                Semantic::Binding(_) => "resolves.binding",
                Semantic::Binds(_) => "binds",
                Semantic::Value(_) => "value",
                Semantic::Arguments(_) => "arguments",
                Semantic::Parse(_) => "parse",
                Semantic::Host(_) => "host",
                Semantic::Capture(_) => "capture",
                Semantic::FormatArgs(_) => "formatArgs",
                Semantic::Layout(_) => "layout",
                Semantic::Calls(_) => "calls",
            }),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => unsupported("testUnknown"),
        }
    }
}
