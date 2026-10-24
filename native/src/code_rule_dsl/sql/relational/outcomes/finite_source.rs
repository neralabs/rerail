//! Non-mutating cycle proof and fallback-only finite predicate fusion.
use super::super::super::ir::Set;
use super::super::{Op, Plan, Slots};
use super::{Lower, Rc};
use crate::code_rule_dsl::matcher::{AstMatcher, Semantic, StopBy, UtilRef};
use std::collections::HashSet;

/// True means the root utility belongs to a cycle, including paths through
/// semantic child programs. Acyclic wrappers around a cyclic helper stay finite.
fn cyclic(root: &UtilRef) -> bool {
    let root_key = (root.table.as_ptr() as usize, root.index);
    let mut seen = HashSet::new();
    fn rule(m: &AstMatcher, root: (usize, usize), seen: &mut HashSet<(usize, usize)>) -> bool {
        match m {
            AstMatcher::Util(u) => reference(u, root, seen),
            AstMatcher::Call(c) => reference(&c.util, root, seen),
            AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().any(|m| rule(m, root, seen)),
            AstMatcher::Not(m) => rule(m, root, seen),
            AstMatcher::Inside(c, s)
            | AstMatcher::Has(c, s)
            | AstMatcher::Follows(c, s)
            | AstMatcher::Precedes(c, s) => {
                rule(c.rule(), root, seen) || matches!(s,StopBy::Rule(m) if rule(m,root,seen))
            }
            AstMatcher::Field(f) => {
                rule(&f.rule, root, seen) || matches!(&f.stop,StopBy::Rule(m) if rule(m,root,seen))
            }
            AstMatcher::Constrained(c) => {
                rule(&c.rule, root, seen) || c.constraints.iter().any(|(_, m)| rule(m, root, seen))
            }
            AstMatcher::Count(c) => rule(&c.relation, root, seen),
            AstMatcher::NthChild(c) => c.of_rule.iter().any(|m| rule(m, root, seen)),
            AstMatcher::Semantic(s) => match s {
                Semantic::Binding(t) => t.sub_rules().any(|m| rule(m, root, seen)),
                Semantic::Binds(t) => t.sub_rules().any(|m| rule(m, root, seen)),
                Semantic::Parse(t) => rule(&t.rule, root, seen),
                Semantic::Host(t) => rule(&t.rule, root, seen),
                Semantic::Calls(_) => true, // No finite call-graph child contract here.
                Semantic::FormatArgs(t) => [t.unused.as_ref(), t.missing.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|m| rule(m, root, seen)),
                _ => false,
            },
            _ => false,
        }
    }
    fn reference(u: &UtilRef, root: (usize, usize), seen: &mut HashSet<(usize, usize)>) -> bool {
        let key = (u.table.as_ptr() as usize, u.index);
        key == root || (seen.insert(key) && u.with_target(|m| rule(m, root, seen)).unwrap_or(true))
    }
    root.with_target(|m| rule(m, root_key, &mut seen))
        .unwrap_or(true)
}
impl Lower {
    fn finite_key(&self, util: &UtilRef) -> super::ScopedUtility {
        (
            util.table.as_ptr() as usize,
            util.index,
            self.namespace.clone(),
            self.parsed_depth > 0,
            self.hosted_depth > 0,
        )
    }
    pub(super) fn finite_cached(&self, util: &UtilRef) -> Option<Plan> {
        self.finite_mode
            .then(|| self.finite_cache.get(&self.finite_key(util)).cloned())
            .flatten()
    }
    pub(super) fn finite_remember(&mut self, matcher: &AstMatcher, plan: &Plan) {
        if self.finite_mode
            && let AstMatcher::Util(util) = matcher
        {
            self.finite_cache
                .insert(self.finite_key(util), plan.clone());
        }
    }

    pub(super) fn finite_acyclic(&self, util: &UtilRef) -> bool {
        self.finite_mode && !cyclic(util)
    }
    pub(super) fn finite_utility(&mut self, util: &UtilRef) -> Result<Plan, String> {
        let key = (util.table.as_ptr() as usize, util.index);
        let scoped = (
            key.0,
            key.1,
            format!(
                "{}@parsed={}@hosted={}",
                self.namespace,
                self.parsed_depth > 0,
                self.hosted_depth > 0
            ),
        );
        let (id, body) = if let Some((id, body)) = self.utilities.get(&scoped) {
            (*id, Rc::clone(body))
        } else {
            if !self.active.insert(key) {
                return Err("finite control: unproved cyclic utility".into());
            }
            let result = util.with_target(|m| self.plan(m));
            self.active.remove(&key);
            let body = Rc::new(result.ok_or("finite control: missing utility")??);
            let id = self.utilities.len();
            self.utilities.insert(scoped, (id, Rc::clone(&body)));
            let scope = util
                .scope()
                .flatten()
                .map(|names| names.iter().map(|name| name.to_string()).collect());
            self.utility_origins.insert(id, (key, scope));
            (id, body)
        };
        let mentions = body.mentions.clone();
        let writes = body.writes.clone();
        let node_writes = body.node_writes.clone();
        let op = if self.parsed_depth > 0 || self.hosted_depth > 0 {
            Op::TransientCharge(body)
        } else {
            Op::Charge(id, body)
        };
        Ok(Plan {
            effects: super::super::effects::Effects::of(&op),
            op,
            mentions,
            writes,
            node_writes,
            walk_filter: None,
        })
    }
    fn finite_set(&mut self, set: &Set) -> Result<(), String> {
        if self.finite_predicates.contains(set) {
            return Ok(());
        }
        match set {
            Set::Intersect(items) | Set::Union(items) => {
                for item in items {
                    self.finite_set(item)?;
                }
            }
            Set::Complement(item) | Set::Preimage(_, item) => self.finite_set(item)?,
            _ => {}
        }
        self.operators += 1;
        if self.operators > 256 && !self.source_inventory_only() {
            return Err("finite control: distinct normalized source operators exceed256".into());
        }
        self.finite_predicates.push(set.clone());
        Ok(())
    }
    pub(super) fn finite_predicate(
        &mut self,
        matcher: &AstMatcher,
    ) -> Result<Option<Plan>, String> {
        if !self.finite_mode || super::super::super::recursive_chain::pure(matcher).is_err() {
            return Ok(None);
        }
        let set = normalize(self.pure.set(matcher)?);
        self.finite_set(&set)?;
        let op = Op::Predicate(set);
        Ok(Some(Plan {
            effects: super::super::effects::Effects::of(&op),
            op,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        }))
    }
}

/// Total predicates admit idempotence and constant folding without crossing any
/// charged, capture-dependent, or three-valued operation. The normalized Set
/// DAG, including its nested definitions, remains under the source budget.
fn normalize(set: Set) -> Set {
    let all = matches!(set, Set::Intersect(_));
    match set {
        Set::Intersect(items) | Set::Union(items) => {
            let mut result = vec![];
            for item in items {
                let item = normalize(item);
                if (all && item == Set::Empty) || (!all && item == Set::AllNodes) {
                    return item;
                }
                if (all && item == Set::AllNodes) || (!all && item == Set::Empty) {
                    continue;
                }
                let nested = match item {
                    Set::Intersect(v) if all => v,
                    Set::Union(v) if !all => v,
                    other => vec![other],
                };
                for item in nested {
                    if !result.contains(&item) {
                        result.push(item);
                    }
                }
            }
            if result.is_empty() {
                if all { Set::AllNodes } else { Set::Empty }
            } else if result.len() == 1 {
                result.pop().unwrap()
            } else if all {
                Set::Intersect(result)
            } else {
                Set::Union(result)
            }
        }
        Set::Complement(item) => match normalize(*item) {
            Set::AllNodes => Set::Empty,
            Set::Empty => Set::AllNodes,
            Set::Complement(inner) => *inner,
            other => Set::Complement(Box::new(other)),
        },
        Set::Preimage(edge, item) => match normalize(*item) {
            Set::Empty => Set::Empty,
            other => Set::Preimage(edge, Box::new(other)),
        },
        other => other,
    }
}
