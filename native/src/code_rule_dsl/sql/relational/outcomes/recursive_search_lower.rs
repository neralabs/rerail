//! Ordered semantic alternatives with strict structural self-call selectors.
use super::{Axis, Lower, Op, Plan, Rc, Slots};
use crate::code_rule_dsl::{
    matcher::{self, AstMatcher, StopBy, UtilRef},
    sql::relational::{ir::Set, recursive_chain::pure, recursive_downward::Downward},
};

pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Arm {
    Base(usize),
    Search {
        axis: Axis,
        guard: Set,
        child_guard: Option<Set>,
        child_kinds: Option<Vec<u16>>,
        field: Option<u16>,
    },
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct RecursiveSearch {
    pub base: Plan,
    pub arms: Vec<Arm>,
    pub memo: bool,
    pub bypass: bool,
    pub guard: Set,
    pub ordered_bases: bool,
    pub child_kinds: Option<Vec<u16>>,
}
struct Tail<'a> {
    axis: Axis,
    field: Option<u16>,
    guards: &'a [AstMatcher],
    child_guards: &'a [AstMatcher],
    target: &'a AstMatcher,
}
impl Lower {
    pub(super) fn recursive_search(&mut self, util: &UtilRef) -> Result<Option<Op>, String> {
        let key = (util.table.as_ptr() as usize, util.index);
        let scoped = (
            key.0,
            key.1,
            self.namespace.clone(),
            self.parsed_depth > 0,
            self.hosted_depth > 0,
        );
        if let Some((id, component)) = self.recursive_searches.get(&scoped) {
            return Ok(Some(Op::RecursiveSearch {
                id: *id,
                component: Rc::clone(component),
            }));
        }
        util.with_target(|body| {
            let AstMatcher::Any(items) = body else {
                return Ok(None);
            };
            if items.len() < 2 {
                return Ok(None);
            }
            let parsed: Vec<_> = items.iter().map(|rule| tail(rule, key)).collect();
            let positions: Vec<_> = parsed
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.as_ref().map(|_| i))
                .collect();
            if positions.is_empty() {
                return Ok(None);
            }
            // Every cycle is a self call below a strict edge. All alternatives
            // move in the same preorder direction; no parent/child oscillation.
            let positive = |axis| matches!(axis, Axis::Child | Axis::Next);
            let direction = positive(parsed[positions[0]].as_ref().unwrap().axis);
            if parsed
                .iter()
                .flatten()
                .any(|t| positive(t.axis) != direction)
            {
                return Ok(None);
            }
            let last = parsed[*positions.last().unwrap()].as_ref().unwrap();
            let legacy = positions.len() == 1
                && positions[0] + 1 == items.len()
                && matches!(last.axis, Axis::Child)
                && last.field.is_none()
                && last.child_guards.is_empty();
            let mut arms = Vec::new();
            let mut base_index = 0;
            for parsed in &parsed {
                if let Some(tail) = parsed {
                    let guard = Set::Intersect(
                        tail.guards
                            .iter()
                            .map(|g| self.pure.set(g))
                            .collect::<Result<_, _>>()?,
                    );
                    let child_guard = if tail.child_guards.is_empty() {
                        None
                    } else {
                        Some(Set::Intersect(
                            tail.child_guards
                                .iter()
                                .map(|g| self.pure.set(g))
                                .collect::<Result<_, _>>()?,
                        ))
                    };
                    let child_kinds = if self.three_valued {
                        None
                    } else {
                        matcher::walk_candidate_filter(tail.target).map(|(k, _)| k)
                    };
                    arms.push(Arm::Search {
                        axis: tail.axis,
                        guard,
                        child_guard,
                        child_kinds,
                        field: tail.field,
                    });
                } else {
                    arms.push(Arm::Base(base_index));
                    base_index += 1;
                }
            }

            let base_count = items.len() - positions.len();
            if base_count > 1 {
                self.operators += 1;
                if self.operators > 256 {
                    return Err(
                        "outcome relations: distinct lowered operator count exceeds 256".into(),
                    );
                }
            }
            if !self.active.insert(key) {
                return Err("recursive search: cyclic base dependency".into());
            }
            // Keep the actual source references. Cloning a synthetic AST Any
            // would change binding-following and parameter-call origin addresses.
            let mut bases = Vec::new();
            let mut result = Ok(());
            for (rule, parsed) in items.iter().zip(&parsed) {
                if parsed.is_none() {
                    match self.plan(rule) {
                        Ok(plan) => bases.push(plan),
                        Err(error) => {
                            result = Err(error);
                            break;
                        }
                    }
                }
            }
            self.active.remove(&key);
            result?;
            let base = if bases.len() == 1 {
                bases.pop().unwrap()
            } else {
                let mut mentions = Slots::new();
                let mut writes = Slots::new();
                let mut node_writes = Slots::new();
                for plan in &bases {
                    mentions.extend(&plan.mentions);
                    writes.extend(&plan.writes);
                    node_writes.extend(&plan.node_writes);
                }
                let op = Op::Any(bases);
                let effects = super::super::effects::Effects::of(&op);
                Plan {
                    op,
                    effects,
                    mentions,
                    writes,
                    node_writes,
                    walk_filter: None,
                }
            };
            let ordered_bases = base_count > 1;
            let memo = !legacy
                || base.uses_memo()
                || (self.three_valued && ordered_bases && !base.writes.is_empty())
                || matcher::reads_values(&AstMatcher::Util(util.clone()), &mut Default::default());
            let (guard, child_kinds) = match &arms[positions[0]] {
                Arm::Search {
                    guard, child_kinds, ..
                } => (guard.clone(), child_kinds.clone()),
                _ => unreachable!(),
            };
            if !memo
                && (Downward {
                    base: Set::Empty,
                    guard: guard.clone(),
                    child_kinds: None,
                })
                .guard_kinds()
                .is_none()
            {
                return Ok(None);
            }
            let id = self.recursive_searches.len();
            if memo {
                let scope = util
                    .scope()
                    .flatten()
                    .map(|names| names.iter().map(|name| name.to_string()).collect());
                self.utility_origins.insert(2_000_000 + id, (key, scope));
            }
            let component = Rc::new(RecursiveSearch {
                base,
                arms,
                memo,
                bypass: self.parsed_depth > 0 || self.hosted_depth > 0,
                guard,
                ordered_bases,
                child_kinds,
            });
            self.recursive_searches
                .insert(scoped, (id, Rc::clone(&component)));
            Ok(Some(Op::RecursiveSearch { id, component }))
        })
        .unwrap_or(Ok(None))
    }
}
/// Pure prefixes can reject a selector before its utility charge. No suffix or
/// semantic effect is moved across the recursive call.
fn tail(rule: &AstMatcher, key: (usize, usize)) -> Option<Tail<'_>> {
    let (guards, walk) = match rule {
        AstMatcher::All(items) if !items.is_empty() => (&items[..items.len() - 1], items.last()?),
        other => (&[][..], other),
    };
    let (axis, field, target) = match walk {
        AstMatcher::Has(child, StopBy::Neighbor) => (Axis::Child, None, child.rule()),
        AstMatcher::Inside(child, StopBy::Neighbor) => (Axis::Parent, None, child.rule()),
        AstMatcher::Follows(child, StopBy::Neighbor) => (Axis::Previous, None, child.rule()),
        AstMatcher::Precedes(child, StopBy::Neighbor) => (Axis::Next, None, child.rule()),
        AstMatcher::Field(field) if matches!(field.stop, StopBy::Neighbor) => (
            if field.inside {
                Axis::Parent
            } else {
                Axis::Child
            },
            Some(field.field),
            &field.rule,
        ),
        _ => return None,
    };
    let (child_guards, call) = match target {
        AstMatcher::All(items) if !items.is_empty() => (&items[..items.len() - 1], items.last()?),
        other => (&[][..], other),
    };
    let AstMatcher::Util(next) = call else {
        return None;
    };
    if (next.table.as_ptr() as usize, next.index) != key
        || guards.iter().chain(child_guards).any(|g| pure(g).is_err())
    {
        return None;
    }
    Some(Tail {
        axis,
        field,
        guards,
        child_guards,
        target,
    })
}

/// Read-only admission shape for the mixed compiler's singleton deferral.
/// The caller must additionally prove that each base arm is SCC-free.
pub(super) fn singleton_arms(body: &AstMatcher, key: (usize, usize)) -> Option<Vec<bool>> {
    let AstMatcher::Any(items) = body else {
        return None;
    };
    if items.len() < 2 {
        return None;
    }
    let tails: Vec<_> = items.iter().map(|rule| tail(rule, key)).collect();
    let mut directions = tails
        .iter()
        .flatten()
        .map(|tail| matches!(tail.axis, Axis::Child | Axis::Next));
    let direction = directions.next()?;
    if directions.any(|other| other != direction) {
        return None;
    }
    Some(tails.iter().map(Option::is_some).collect())
}
