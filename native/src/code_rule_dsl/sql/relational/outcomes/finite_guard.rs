//! Zero-effect rejection prefixes for finite SQL control. A false guard proves
//! rejection before any utility entry, capture operation, regex or native fact.
use super::super::ir::{Edge, Set};
use super::finite_lower::{Instruction, Program};
use super::{Axis, Op, Plan};
use std::{collections::HashMap, rc::Rc};

const LIMIT: usize = 256;
#[derive(Clone)]
struct Proof {
    guard: Set,
    complete: bool,
    size: usize,
}
#[derive(Clone)]
enum Memo {
    Visiting,
    Ready(Option<Rc<Proof>>),
}
struct Derive<'a> {
    program: &'a Program,
    code: Vec<Option<Memo>>,
    plans: HashMap<usize, Memo>,
    remaining: usize,
    cyclic: bool,
}
/// Only a structural preimage can improve a root already filtered by anchors.
/// Guard derivation is optional: unsupported/oversize/cyclic input keeps the
/// original machine, never changes admission or its native work contract.
pub(super) fn entry(program: &Program) -> Option<Set> {
    if program.code.len() > LIMIT {
        return None;
    }
    let mut derive = Derive {
        program,
        code: vec![None; program.code.len()],
        plans: HashMap::new(),
        remaining: LIMIT,
        cyclic: false,
    };
    let proof = derive.code(program.entry)?;
    (!derive.cyclic && preimage(&proof.guard)).then(|| proof.guard.clone())
}
fn preimage(set: &Set) -> bool {
    match set {
        Set::Preimage(..) => true,
        Set::Intersect(items) | Set::Union(items) => items.iter().any(preimage),
        Set::Complement(child) => preimage(child),
        _ => false,
    }
}
fn proof(guard: Set, complete: bool, size: usize) -> Rc<Proof> {
    Rc::new(Proof {
        guard,
        complete,
        size,
    })
}
fn combine(all: bool, items: Vec<Rc<Proof>>, complete: bool) -> Option<Rc<Proof>> {
    let size = items
        .iter()
        .try_fold(1usize, |size, p| size.checked_add(p.size))
        .filter(|n| *n <= LIMIT)?;
    let sets = items
        .into_iter()
        .map(|p| p.guard.clone())
        .collect::<Vec<_>>();
    Some(proof(
        if all {
            Set::Intersect(sets)
        } else {
            Set::Union(sets)
        },
        complete,
        size,
    ))
}
impl Derive<'_> {
    fn tick(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }
    fn code(&mut self, pc: usize) -> Option<Rc<Proof>> {
        match self.code.get(pc)?.clone() {
            Some(Memo::Visiting) => {
                self.cyclic = true;
                return None;
            }
            Some(Memo::Ready(value)) => return value,
            None => {}
        }
        self.tick()?;
        self.code[pc] = Some(Memo::Visiting);
        let program = self.program;
        let result = match &program.code[pc].op {
            Instruction::Eval(id) => self.plan(program.regions.get(*id)?),
            Instruction::Boolean { all, children } => {
                self.boolean(*all, children, |this, pc| this.code(*pc))
            }
            Instruction::Not(child) => self.code(*child).and_then(not),
            Instruction::Search {
                axis,
                end,
                field,
                body,
                stop,
                bounds,
                filter,
            } if !*end && field.is_none() && stop.is_none() && bounds.is_none() => {
                if filter.as_ref().is_some_and(|f| !f.text.is_empty()) {
                    None
                } else {
                    let body = self.code(*body);
                    let body = if let Some(filter) = filter {
                        let kind = proof(Set::Kinds(filter.kinds.clone()), true, 1);
                        match body {
                            Some(body) => {
                                let complete = body.complete;
                                combine(true, vec![kind, body], complete)
                            }
                            None => Some(proof(Set::Kinds(filter.kinds.clone()), false, 1)),
                        }
                    } else {
                        body
                    };
                    body.and_then(|body| neighbor(*axis, body))
                }
            }
            // Every Call spends utility work before its body. Stops can run
            // before the relation body, and counts need different truth algebra.
            Instruction::Call(_) | Instruction::Search { .. } | Instruction::Binding { .. } => None,
        };
        self.code[pc] = Some(Memo::Ready(result.clone()));
        result
    }
    fn plan(&mut self, plan: &Plan) -> Option<Rc<Proof>> {
        let id = std::ptr::from_ref(plan) as usize;
        match self.plans.get(&id).cloned() {
            Some(Memo::Visiting) => {
                self.cyclic = true;
                return None;
            }
            Some(Memo::Ready(value)) => return value,
            None => {}
        }
        self.tick()?;
        self.plans.insert(id, Memo::Visiting);
        let result = match &plan.op {
            Op::Predicate(set) => self.metadata(set),
            Op::Shared(child) => self.plan(child),
            Op::All(children) => self.boolean(true, children, |this, child| this.plan(child)),
            Op::Any(children) => self.boolean(false, children, |this, child| this.plan(child)),
            Op::Not(child) => self.plan(child).and_then(not),
            Op::Search(axis, false, body, None) => {
                self.plan(body).and_then(|body| neighbor(*axis, body))
            }
            _ => None,
        };
        self.plans.insert(id, Memo::Ready(result.clone()));
        result
    }
    fn boolean<T>(
        &mut self,
        all: bool,
        children: &[T],
        mut child: impl FnMut(&mut Self, &T) -> Option<Rc<Proof>>,
    ) -> Option<Rc<Proof>> {
        let mut proofs = vec![];
        let mut complete = true;
        let mut size = 1usize;
        for item in children {
            let Some(next) = child(self, item) else {
                if !all {
                    return None;
                }
                complete = false;
                break;
            };
            let Some(next_size) = size.checked_add(next.size).filter(|n| *n <= LIMIT) else {
                if !all {
                    return None;
                }
                complete = false;
                break;
            };
            size = next_size;
            let partial = !next.complete;
            proofs.push(next);
            if all && partial {
                complete = false;
                break;
            }
        }
        if proofs.is_empty() && !children.is_empty() {
            return None;
        }
        combine(all, proofs, complete)
    }
    fn metadata(&mut self, set: &Set) -> Option<Rc<Proof>> {
        self.tick()?;
        match set {
            Set::AllNodes | Set::Empty | Set::Kinds(_) => Some(proof(set.clone(), true, 1)),
            Set::Intersect(items) | Set::Union(items) => {
                let proofs = items
                    .iter()
                    .map(|item| self.metadata(item))
                    .collect::<Option<Vec<_>>>()?;
                combine(matches!(set, Set::Intersect(_)), proofs, true)
            }
            Set::Complement(child) => self.metadata(child).and_then(not),
            Set::Preimage(edge, child) => self.metadata(child).and_then(|child| {
                child
                    .size
                    .checked_add(1)
                    .filter(|size| *size <= LIMIT)
                    .map(|size| {
                        proof(
                            Set::Preimage(*edge, Box::new(child.guard.clone())),
                            child.complete,
                            size,
                        )
                    })
            }),
            _ => None,
        }
    }
}
fn not(child: Rc<Proof>) -> Option<Rc<Proof>> {
    if !child.complete {
        return None;
    }
    let size = child.size.checked_add(1).filter(|n| *n <= LIMIT)?;
    Some(proof(
        Set::Complement(Box::new(child.guard.clone())),
        true,
        size,
    ))
}
fn neighbor(axis: Axis, child: Rc<Proof>) -> Option<Rc<Proof>> {
    let size = child.size.checked_add(1).filter(|n| *n <= LIMIT)?;
    let edge = match axis {
        Axis::Child => Edge::Child,
        Axis::Parent => Edge::Parent,
        Axis::Previous => Edge::Previous,
        Axis::Next => Edge::Next,
    };
    Some(proof(
        Set::Preimage(edge, Box::new(child.guard.clone())),
        child.complete,
        size,
    ))
}

#[cfg(test)]
#[path = "finite_guard_tests.rs"]
mod tests;
