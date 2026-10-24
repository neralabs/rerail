//! Cache costly predicate helpers only where different origins can revisit a node.
//! The proof excludes every utility/value/typed callback: cost and peak are one.
use super::super::ir::Set;
use super::{Axis, Op, Plan, effects::Effects};
use std::{collections::HashMap, rc::Rc};

fn predicate(set: &Set) -> bool {
    match set {
        Set::AllNodes
        | Set::Empty
        | Set::Kinds(_)
        | Set::Regex(_)
        | Set::Nonempty
        | Set::Pattern(_) => true,
        Set::Intersect(items) | Set::Union(items) => items.iter().all(predicate),
        Set::Complement(p) | Set::Preimage(_, p) => predicate(p),
        _ => false,
    }
}
fn utility_free(plan: &Plan) -> bool {
    match &plan.op {
        Op::Predicate(p) => predicate(p),
        Op::All(items) | Op::Any(items) => items.iter().all(utility_free),
        Op::Not(p) => utility_free(p),
        Op::Search(_, _, child, stop) => {
            utility_free(child) && stop.as_ref().is_none_or(|p| utility_free(p))
        }
        Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => utility_free(child),
        Op::FieldRule { child, stop, .. } => utility_free(child) && utility_free(stop),
        _ => false,
    }
}
pub(super) fn eligible(plan: &Plan) -> bool {
    !plan.fallible()
        && !plan.effects.contains_chain
        && !plan.reads_value()
        && !plan.effects.cache_fills
        && plan.mentions.is_empty()
        && plan.writes.is_empty()
        && plan.effects.walk_depth >= 2
        && utility_free(plan)
}
pub(super) fn prepare(plan: Plan) -> Plan {
    Pass::default().transform(&plan, false).0
}
#[derive(Default)]
struct Pass {
    bodies: HashMap<(usize, bool), (Rc<Plan>, bool)>,
}
impl Pass {
    fn body(&mut self, body: &Rc<Plan>, reused: bool) -> (Rc<Plan>, bool) {
        let key = (Rc::as_ptr(body) as usize, reused);
        if let Some(found) = self.bodies.get(&key) {
            return found.clone();
        }
        let (next, did) = self.transform(body, reused);
        let result = (if did { Rc::new(next) } else { Rc::clone(body) }, did);
        self.bodies.insert(key, result.clone());
        result
    }
    fn child(&mut self, plan: &Plan, reused: bool, changed: &mut bool) -> Plan {
        let (next, did) = self.transform(plan, reused);
        *changed |= did;
        next
    }
    fn transform(&mut self, plan: &Plan, reused: bool) -> (Plan, bool) {
        let mut changed = false;
        let op = match &plan.op {
            Op::Charge(id, body) => {
                if reused && eligible(body) {
                    let mut out = plan.clone();
                    out.op = Op::MemoPredicate(*id, Rc::clone(body));
                    out.effects = Effects::of(&out.op);
                    return (out, true);
                }
                let (next, did) = self.body(body, reused);
                changed |= did;
                Op::Charge(*id, next)
            }
            Op::Shared(body) => {
                let (next, did) = self.body(body, reused);
                changed |= did;
                Op::Shared(next)
            }
            Op::All(items) => Op::All(
                items
                    .iter()
                    .map(|p| self.child(p, reused, &mut changed))
                    .collect(),
            ),
            Op::Any(items) => Op::Any(
                items
                    .iter()
                    .map(|p| self.child(p, reused, &mut changed))
                    .collect(),
            ),
            Op::Not(p) => Op::Not(Box::new(self.child(p, reused, &mut changed))),
            Op::Search(axis, end, p, stop) => {
                let repeat = reused || matches!(axis, Axis::Parent) || *end;
                Op::Search(
                    *axis,
                    *end,
                    Box::new(self.child(p, repeat, &mut changed)),
                    stop.as_ref()
                        .map(|p| Box::new(self.child(p, repeat, &mut changed))),
                )
            }
            Op::FieldNeighbor {
                inside,
                field,
                child: p,
            } => Op::FieldNeighbor {
                inside: *inside,
                field: *field,
                child: Box::new(self.child(p, reused || *inside, &mut changed)),
            },
            Op::FieldEnd {
                inside,
                field,
                child: p,
            } => Op::FieldEnd {
                inside: *inside,
                field: *field,
                child: Box::new(self.child(p, reused || *inside, &mut changed)),
            },
            Op::FieldRule {
                inside,
                field,
                child: p,
                stop,
            } => Op::FieldRule {
                inside: *inside,
                field: *field,
                child: Box::new(self.child(p, true, &mut changed)),
                stop: Box::new(self.child(stop, true, &mut changed)),
            },
            Op::Constrained(core, rules) => Op::Constrained(
                Box::new(self.child(core, reused, &mut changed)),
                rules
                    .iter()
                    .map(|(s, p)| (*s, self.child(p, true, &mut changed)))
                    .collect(),
            ),
            Op::Count {
                axis,
                end,
                rule,
                stop,
                min,
                max,
                binding,
            } => {
                let repeat = reused || matches!(axis, Axis::Parent) || *end;
                Op::Count {
                    axis: *axis,
                    end: *end,
                    rule: Box::new(self.child(rule, repeat, &mut changed)),
                    stop: stop
                        .as_ref()
                        .map(|p| Box::new(self.child(p, repeat, &mut changed))),
                    min: *min,
                    max: *max,
                    binding: *binding,
                }
            }
            _ => return (plan.clone(), false),
        };
        if !changed {
            return (plan.clone(), false);
        }
        let mut out = plan.clone();
        out.op = op;
        out.effects = Effects::of(&out.op);
        (out, true)
    }
}
