//! Expose only already-proved ordered recursive components to finite body sharing.
//! The source lowerer proves every self edge strict and every arm direction equal.
use super::super::{
    Op, Plan, Slots, WalkFilter,
    lower::recursive_search_lower::{Arm, RecursiveSearch},
};
use super::{Builder, Function, Instruction};
use crate::code_rule_dsl::sql::relational::ir::Set;
use std::{collections::HashSet, rc::Rc};
pub(super) fn has_functions(plan: &Plan, seen: &mut HashSet<usize>) -> bool {
    if !seen.insert(std::ptr::from_ref(plan) as usize) {
        return false;
    }
    match &plan.op {
        Op::Charge(..) | Op::MemoPredicate(..) | Op::TransientCharge(_) => true,
        Op::Shared(p) => has_functions(p, seen),
        Op::Not(p) => has_functions(p, seen),
        Op::All(v) | Op::Any(v) => v.iter().any(|p| has_functions(p, seen)),
        Op::FieldNeighbor { child, .. }
        | Op::FieldEnd { child, .. }
        | Op::Parse { child, .. }
        | Op::Host { child, .. } => has_functions(child, seen),
        Op::FieldRule { child, stop, .. } => {
            has_functions(child, seen) || has_functions(stop, seen)
        }
        Op::Search(_, _, p, s)
        | Op::Count {
            rule: p, stop: s, ..
        } => has_functions(p, seen) || s.as_ref().is_some_and(|p| has_functions(p, seen)),
        Op::Binding {
            core,
            children,
            occurrence,
            ..
        } => {
            has_functions(core, seen)
                || children.iter().any(|(_, p)| has_functions(p, seen))
                || occurrence.as_ref().is_some_and(|p| has_functions(p, seen))
        }
        Op::RecursiveSearch { component, .. } => has_functions(&component.base, seen),
        Op::Constrained(p, v) => {
            has_functions(p, seen) || v.iter().any(|(_, p)| has_functions(p, seen))
        }
        Op::NthChild {
            of_rule: Some(p), ..
        } => has_functions(p, seen),
        _ => false,
    }
}
fn simplify(set: &Set) -> Set {
    match set {
        Set::Intersect(v) if v.is_empty() => Set::AllNodes,
        Set::Union(v) if v.is_empty() => Set::Empty,
        Set::Intersect(v) | Set::Union(v) if v.len() == 1 => simplify(&v[0]),
        other => other.clone(),
    }
}
impl Builder {
    fn recursive_predicate(&mut self, set: &Set) -> usize {
        let plan = Plan {
            op: Op::Predicate(simplify(set)),
            effects: Default::default(),
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        };
        self.region(&plan)
    }
    pub(super) fn recursive(
        &mut self,
        id: usize,
        component: &Rc<RecursiveSearch>,
        plan: &Plan,
    ) -> Result<usize, String> {
        let key = (id, Rc::as_ptr(component) as usize);
        if let Some(&function) = self.recursive_functions.get(&key) {
            return Ok(self.call_nodes[&function]);
        }
        let function = self.functions.len();
        self.functions.push(Function {
            entry: usize::MAX,
            original: None,
            memo: (component.memo && !component.bypass).then_some(2_000_000 + id),
            effects: plan.effects,
            mentions: plan.mentions.clone(),
            writes: plan.writes.clone(),
        });
        self.recursive_functions.insert(key, function);
        // Only the typed strict-progress component can create a self call. The
        // ordinary utility builder still refuses unproved graph cycles.
        let recurse = self.node(plan, Instruction::Call(function));
        self.call_nodes.insert(function, recurse);
        let bases = if component.ordered_bases {
            let Op::Any(v) = &component.base.op else {
                return Err("finite recursion: missing ordered base arms".into());
            };
            v.iter().collect::<Vec<_>>()
        } else {
            vec![&component.base]
        };
        let mut arms = vec![];
        for arm in &component.arms {
            match arm {
                Arm::Base(index) => arms.push(
                    self.plan(
                        bases
                            .get(*index)
                            .ok_or("finite recursion: invalid base arm")?,
                    )?,
                ),
                Arm::Search {
                    axis,
                    guard,
                    child_guard,
                    child_kinds,
                    field,
                } => {
                    let body = if let Some(child) = child_guard
                        && simplify(child) != Set::AllNodes
                    {
                        let predicate = self.recursive_predicate(child);
                        self.node(
                            plan,
                            Instruction::Boolean {
                                all: true,
                                children: vec![predicate, recurse],
                            },
                        )
                    } else {
                        recurse
                    };
                    let search = self.node(
                        plan,
                        Instruction::Search {
                            axis: *axis,
                            end: false,
                            field: *field,
                            body,
                            stop: None,
                            bounds: None,
                            filter: child_kinds.as_ref().map(|kinds| WalkFilter {
                                kinds: kinds.clone(),
                                text: vec![],
                            }),
                        },
                    );
                    if simplify(guard) == Set::AllNodes {
                        arms.push(search);
                    } else {
                        let predicate = self.recursive_predicate(guard);
                        arms.push(self.node(
                            plan,
                            Instruction::Boolean {
                                all: true,
                                children: vec![predicate, search],
                            },
                        ));
                    }
                }
            }
        }
        let entry = self.node(
            plan,
            Instruction::Boolean {
                all: false,
                children: arms,
            },
        );
        self.functions[function].entry = entry;
        Ok(recurse)
    }
}
