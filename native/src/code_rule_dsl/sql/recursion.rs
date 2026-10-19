//! Recursive utilities, read from the compiled `AstMatcher` tree: which utilities reach themselves
//! (a strongly connected component of the utility graph, a `Cycle`) and which captures a
//! utility's scope holds.
//!
//! # Why a fixpoint over a whole batch evaluates a recursive utility exactly
//!
//! A recursive utility is evaluated as units (`plan::Unit`): a unit's answer at a key
//! `(file, node, bindings it reads)` is the bindings after its body, or no match. The driver runs
//! every unit's body of one component over all keys demanded so far, each recursive reference
//! reading the answers of the keys it demands as they stand (none at first), and repeats until a
//! pass changes no answer and demands no key. Every reference a body makes in that last pass is
//! recorded, from the key being evaluated to the key read: the pass's read graph. The driver
//! accepts the answers only where that graph is acyclic (`exec`); a cycle, or a pass count past the
//! matcher's depth limit, refuses the batch.
//!
//! Accepted answers are the matcher's. Let `A` be the last pass's answers: `A = T(A)`, `T` the
//! bodies, and its read graph `G` is acyclic. Take the keys in an order where each comes after
//! every key it reads in `G` (one exists, `G` being acyclic and finite) and argue by induction. A
//! key's body reads only its successors in `G`, whose answers in `A` are the matcher's by
//! induction. The matcher, evaluating the same body at the same key, reads a subset of them (it
//! stops at a relation's first match and at an `all`'s first false term, where the set evaluation
//! tests every candidate; `any` and `all` read the same keys in both), so it terminates and, given
//! the same answers, reaches the same verdict and bindings: every operator's lowering is the
//! matcher's semantics over its input frame. So the key's answer in `A` is the matcher's too.
//!
//! That holds whatever the body applies to its recursive answers: a negation, a count or a
//! first-match argmin over a recursive reference reads answers that are final for the keys it
//! reads, in the order `G` gives, so it is stratified by that order. A recursion through
//! relations that walk away from the node in one direction (`has` descending, `inside` ascending)
//! always yields such an order: its keys are ordered by depth in the tree, the strata are the
//! levels, and the passes needed are at most the tree's height. A recursion whose read graph may
//! loop (an alias walked up from a reference and down again to it) is evaluated alike and
//! accepted exactly where, in the batch, it does not loop.
//!
//! A non-recursive utility reached through two or more references (`repeated`) is a unit too, a
//! component of one member whose body reads no answer of its own: its loop finishes in one pass,
//! and the argument above holds with a read graph that has no edge. Every reference demands its
//! keys and reads their answers, so the body is lowered and emitted once however many references
//! reach it, and equal keys share an answer, as the matcher's per-file utility memo shares it.

use super::plan::{SlotKind, Unsupported};
use super::terminals::pattern_vars;
use crate::code_rule_dsl::matcher::{AstMatcher, Semantic, UtilRef, stop_rule};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// A utility's identity: its table and index.
pub(super) type UtilKey = (usize, usize);

/// The captures a scope holds: name and kind.
type Held = BTreeSet<(String, SlotKind)>;

pub(super) fn util_key(util: &UtilRef) -> UtilKey {
    (util.table.as_ptr() as usize, util.index)
}

/// A strongly connected component of the utility graph: utilities that reach each other.
pub(super) struct Cycle {
    /// The members' keys, by index in their table.
    pub(super) keys: Vec<UtilKey>,
    /// Why the cycle is refused, if it is.
    pub(super) refused: Option<&'static str>,
}

/// The utility references a rule holds, without following them.
#[cfg(test)]
pub(super) fn references_of(rule: &AstMatcher, out: &mut Vec<UtilRef>) {
    references(rule, out);
}

fn references(rule: &AstMatcher, out: &mut Vec<UtilRef>) {
    walk(rule, false, &mut |util, _| out.push(util.clone()));
}

/// Walks `rule`, calling `found(util, stop)` at each utility reference, `stop` where it lies in a
/// stop rule.
fn walk(rule: &AstMatcher, stop: bool, found: &mut dyn FnMut(&UtilRef, bool)) {
    let related =
        |item: &AstMatcher, stop_by: Option<&AstMatcher>, found: &mut dyn FnMut(&UtilRef, bool)| {
            walk(item, stop, found);
            if let Some(stop_by) = stop_by {
                walk(stop_by, true, found);
            }
        };
    match rule {
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                walk(item, stop, found);
            }
        }
        AstMatcher::Not(item) => walk(item, stop, found),
        AstMatcher::Inside(item, by)
        | AstMatcher::Has(item, by)
        | AstMatcher::Precedes(item, by)
        | AstMatcher::Follows(item, by) => related(item.rule(), stop_rule(by), found),
        AstMatcher::Field(relation) => related(&relation.rule, stop_rule(&relation.stop), found),
        AstMatcher::Count(count) => walk(&count.relation, stop, found),
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                walk(rule, stop, found);
            }
        }
        AstMatcher::Constrained(core) => {
            walk(&core.rule, stop, found);
            for (_, constraint) in &core.constraints {
                walk(constraint, stop, found);
            }
        }
        AstMatcher::Util(util) => found(util, stop),
        AstMatcher::Call(call) => found(&call.util, stop),
        AstMatcher::Semantic(semantic) => {
            let mut inner: Vec<&AstMatcher> = Vec::new();
            match semantic {
                Semantic::Binding(test) => inner.extend(test.sub_rules()),
                Semantic::Binds(test) => inner.extend(test.sub_rules()),
                Semantic::FormatArgs(test) => inner.extend(test.unused.iter()),
                Semantic::Parse(test) => inner.push(&test.rule),
                Semantic::Host(test) => inner.push(&test.rule),
                Semantic::Resolves(_)
                | Semantic::Value(_)
                | Semantic::Arguments(_)
                | Semantic::Capture(_)
                | Semantic::Layout(_)
                | Semantic::Calls(_) => {}
            }
            for item in inner {
                walk(item, stop, found);
            }
        }
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

/// The utilities `util` refers to directly.
fn edges_of(util: &UtilRef) -> Vec<UtilRef> {
    let mut out = Vec::new();
    util.with_target(|rule| references(rule, &mut out));
    out
}

/// Every utility reachable from `start` (itself included when it reaches itself).
fn reachable(start: &UtilRef) -> HashMap<UtilKey, UtilRef> {
    let mut seen: HashMap<UtilKey, UtilRef> = HashMap::new();
    let mut stack = edges_of(start);
    while let Some(util) = stack.pop() {
        let key = util_key(&util);
        if seen.contains_key(&key) {
            continue;
        }
        stack.extend(edges_of(&util));
        seen.insert(key, util);
    }
    seen
}

/// The non-recursive utilities `roots` reach through two or more references, counted over
/// `roots` and every utility they reach: each such utility's body is lowered once, as a unit
/// (`Lowerer::shared_unit`), so each reference is lowered once too, and these are exactly the
/// utilities lowering would otherwise copy. A utility with one reference stays inlined.
pub(super) fn repeated(roots: &[&AstMatcher]) -> HashSet<UtilKey> {
    let mut counts: HashMap<UtilKey, usize> = HashMap::new();
    let mut seen: HashSet<UtilKey> = HashSet::new();
    let mut stack: Vec<UtilRef> = Vec::new();
    let mut count = |rule: &AstMatcher, stack: &mut Vec<UtilRef>| {
        walk(rule, false, &mut |util, _| {
            *counts.entry(util_key(util)).or_default() += 1;
            stack.push(util.clone());
        });
    };
    for root in roots {
        count(root, &mut stack);
    }
    while let Some(util) = stack.pop() {
        if seen.insert(util_key(&util)) {
            util.with_target(|rule| count(rule, &mut stack));
        }
    }
    counts
        .into_iter()
        .filter(|&(_, count)| count >= 2)
        .map(|(key, _)| key)
        .collect()
}

impl Cycle {
    /// The cycle `util` belongs to; `None` where it does not reach itself.
    fn of(util: &UtilRef) -> Option<Self> {
        let key = util_key(util);
        let forward = reachable(util);
        if !forward.contains_key(&key) {
            return None;
        }
        let mut members: Vec<UtilRef> = forward
            .values()
            .filter(|other| reachable(other).contains_key(&key))
            .cloned()
            .collect();
        members.sort_by_key(|member| member.index);
        let keys: Vec<UtilKey> = members.iter().map(util_key).collect();
        let set: HashSet<UtilKey> = keys.iter().copied().collect();
        // A stop rule is a per-batch stage, computed before the clause: one that reaches back into
        // the cycle would need the cycle's answers before they exist.
        let mut in_stop = false;
        for member in &members {
            member.with_target(|rule| {
                walk(rule, false, &mut |util, stop| {
                    in_stop |= stop && set.contains(&util_key(util));
                });
            });
        }
        Some(Self {
            keys,
            refused: in_stop.then_some("recursive utility in a stop rule"),
        })
    }
}

/// Which utilities are recursive, worked out once per utility.
#[derive(Default)]
pub(super) struct Recursion {
    cycles: HashMap<UtilKey, Option<Rc<Cycle>>>,
    /// Per utility table: the captures each utility's scope holds (`scope_kinds`).
    scopes: HashMap<usize, Rc<Vec<Held>>>,
}

impl Recursion {
    /// The cycle `util` belongs to; `None` where it is not recursive.
    pub(super) fn cycle(&mut self, util: &UtilRef) -> Option<Rc<Cycle>> {
        let key = util_key(util);
        if let Some(known) = self.cycles.get(&key) {
            return known.clone();
        }
        let cycle = Cycle::of(util).map(Rc::new);
        match &cycle {
            Some(cycle) => {
                for member in &cycle.keys {
                    self.cycles.insert(*member, Some(Rc::clone(cycle)));
                }
            }
            None => {
                self.cycles.insert(key, None);
            }
        }
        cycle
    }

    /// The captures (name and kind) a utility's rule may hold in the scope it runs in: those its
    /// patterns define, its counts bind (`as`), the utilities it shares its scope with hold, and
    /// those its calls return under the caller's names. These are a recursive unit's columns.
    pub(super) fn scope_kinds(&mut self, util: &UtilRef) -> Held {
        let Some(table) = util.table.upgrade() else {
            return BTreeSet::new();
        };
        let address = util.table.as_ptr() as usize;
        let scopes = match self.scopes.get(&address) {
            Some(scopes) => Rc::clone(scopes),
            None => {
                let rules = table.rules.get().or_else(|| table.draft.get());
                let scopes = Rc::new(rules.map_or_else(Vec::new, |rules| table_scopes(rules)));
                self.scopes.insert(address, Rc::clone(&scopes));
                scopes
            }
        };
        scopes.get(util.index).cloned().unwrap_or_default()
    }
}

/// `Recursion::scope_kinds` for every utility of one table, to their joint fixpoint.
fn table_scopes(rules: &[AstMatcher]) -> Vec<Held> {
    let mut scopes: Vec<Held> = vec![BTreeSet::new(); rules.len()];
    loop {
        let mut changed = false;
        for (index, rule) in rules.iter().enumerate() {
            let mut names = BTreeSet::new();
            holds(rule, &scopes, &mut names);
            if names != scopes[index] {
                scopes[index] = names;
                changed = true;
            }
        }
        if !changed {
            return scopes;
        }
    }
}

fn holds(rule: &AstMatcher, scopes: &[Held], names: &mut Held) {
    match rule {
        AstMatcher::Pattern(pattern) => names.extend(pattern_vars(pattern)),
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                holds(item, scopes, names);
            }
        }
        AstMatcher::Not(item) => holds(item, scopes, names),
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => holds(item.rule(), scopes, names),
        AstMatcher::Field(relation) => holds(&relation.rule, scopes, names),
        AstMatcher::Count(count) => {
            if let Some(binding) = &count.binding {
                names.insert((binding.name.clone(), SlotKind::Count));
            }
            holds(&count.relation, scopes, names);
        }
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                holds(rule, scopes, names);
            }
        }
        AstMatcher::Constrained(core) => {
            holds(&core.rule, scopes, names);
            for (_, constraint) in &core.constraints {
                holds(constraint, scopes, names);
            }
        }
        AstMatcher::Util(util) => {
            if let Some(inner) = scopes.get(util.index) {
                names.extend(inner.iter().cloned());
            }
        }
        AstMatcher::Call(call) => {
            if let Some(inner) = scopes.get(call.util.index) {
                for (param, caller) in &call.args {
                    for (name, kind) in inner {
                        if name == param {
                            names.insert((caller.clone(), *kind));
                        }
                    }
                }
            }
        }
        AstMatcher::Semantic(semantic) => match semantic {
            Semantic::Binding(test) => test.sub_rules().for_each(|rule| holds(rule, scopes, names)),
            Semantic::Binds(test) => test.sub_rules().for_each(|rule| holds(rule, scopes, names)),
            Semantic::Arguments(arguments) => names.extend(
                arguments
                    .bind
                    .iter()
                    .map(|(name, _)| (name.clone(), SlotKind::Node)),
            ),
            Semantic::Capture(name) => {
                names.insert((name.clone(), SlotKind::Node));
            }
            Semantic::Calls(test) => names.extend(
                test.chain
                    .iter()
                    .map(|chain| (chain.clone(), SlotKind::Text)),
            ),
            Semantic::FormatArgs(test) => {
                if let Some(rule) = &test.unused {
                    holds(rule, scopes, names);
                }
                names.extend(
                    test.exports
                        .iter()
                        .map(|name| (name.clone(), SlotKind::Text)),
                );
            }
            Semantic::Parse(test) => names.extend(
                test.exports
                    .iter()
                    .map(|name| (name.clone(), SlotKind::Text)),
            ),
            Semantic::Resolves(_)
            | Semantic::Value(_)
            | Semantic::Layout(_)
            | Semantic::Host(_) => {}
        },
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Compare(_) | AstMatcher::Never => {
        }
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

/// Whether `rule` refers, directly or through the utilities it reaches, to one of `keys`.
pub(super) fn reaches(rule: &AstMatcher, keys: &HashSet<UtilKey>) -> bool {
    if keys.is_empty() {
        return false;
    }
    let mut stack = Vec::new();
    references(rule, &mut stack);
    let mut seen = HashSet::new();
    while let Some(util) = stack.pop() {
        let key = util_key(&util);
        if keys.contains(&key) {
            return true;
        }
        if seen.insert(key) {
            stack.extend(edges_of(&util));
        }
    }
    false
}

/// The refusal for a cycle that is not well-founded, if it is not.
pub(super) fn refusal(cycle: &Cycle) -> Result<(), Unsupported> {
    match cycle.refused {
        Some(reason) => Err(Unsupported(reason.to_owned())),
        None => Ok(()),
    }
}
