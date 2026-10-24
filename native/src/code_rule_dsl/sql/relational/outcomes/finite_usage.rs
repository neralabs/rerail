//! Count physical definition edges, not logical utility invocation multiplicity.
use super::{Op, Plan};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};
pub(super) fn count(plan: &Plan) -> HashMap<usize, usize> {
    fn walk(plan: &Plan, uses: &mut HashMap<usize, usize>, seen: &mut HashSet<(u8, usize, usize)>) {
        match &plan.op {
            Op::Shared(body) => {
                let pointer = Rc::as_ptr(body) as usize;
                *uses.entry(pointer).or_default() += 1;
                if seen.insert((0, pointer, 0)) {
                    walk(body, uses, seen);
                }
            }
            Op::Charge(id, body) | Op::MemoPredicate(id, body)
                if seen.insert((1, Rc::as_ptr(body) as usize, *id)) =>
            {
                walk(body, uses, seen)
            }
            Op::TransientCharge(body) if seen.insert((2, Rc::as_ptr(body) as usize, 0)) => {
                walk(body, uses, seen)
            }
            Op::CallFrame { body, .. } if seen.insert((3, Rc::as_ptr(body) as usize, 0)) => {
                walk(body, uses, seen)
            }
            Op::RecursiveSearch { component, .. }
                if seen.insert((4, Rc::as_ptr(component) as usize, 0)) =>
            {
                walk(&component.base, uses, seen)
            }
            Op::MixedRecursive { program, entry }
                if seen.insert((5, Rc::as_ptr(program) as usize, *entry)) =>
            {
                for p in &program.regions {
                    walk(p, uses, seen)
                }
            }
            Op::ParsedRecursive { program, entry }
                if seen.insert((6, Rc::as_ptr(program) as usize, *entry)) =>
            {
                for p in &program.regions {
                    walk(p, uses, seen)
                }
            }
            Op::FiniteProgram(program) if seen.insert((7, Rc::as_ptr(program) as usize, 0)) => {
                for p in &program.regions {
                    walk(p, uses, seen)
                }
            }
            Op::All(v) | Op::Any(v) => {
                for p in v {
                    walk(p, uses, seen)
                }
            }
            Op::Not(p)
            | Op::Parse { child: p, .. }
            | Op::Host { child: p, .. }
            | Op::FieldNeighbor { child: p, .. }
            | Op::FieldEnd { child: p, .. } => walk(p, uses, seen),
            Op::FieldRule { child, stop, .. } => {
                walk(child, uses, seen);
                walk(stop, uses, seen);
            }
            Op::Search(_, _, p, stop) | Op::Count { rule: p, stop, .. } => {
                walk(p, uses, seen);
                if let Some(p) = stop {
                    walk(p, uses, seen);
                }
            }
            Op::Constrained(p, v) => {
                walk(p, uses, seen);
                for (_, p) in v {
                    walk(p, uses, seen);
                }
            }
            Op::NthChild {
                of_rule: Some(p), ..
            } => walk(p, uses, seen),
            Op::Binding {
                core,
                children,
                occurrence,
                ..
            } => {
                walk(core, uses, seen);
                for (_, p) in children {
                    walk(p, uses, seen);
                }
                if let Some(p) = occurrence {
                    walk(p, uses, seen);
                }
            }
            Op::FormatArgs {
                unused, missing, ..
            } => {
                for p in [unused, missing].into_iter().flatten() {
                    walk(p, uses, seen)
                }
            }
            _ => {}
        }
    }
    let mut uses = HashMap::new();
    walk(plan, &mut uses, &mut HashSet::new());
    uses
}
