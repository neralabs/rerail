//! Shared finite control. SQL evaluates regions lazily; utility bodies have one
//! physical definition and retain their native memo identities at call entry.
use super::{Axis, Plan, Slots, WalkFilter};

pub(super) struct Node {
    pub op: Instruction,
    pub effects: super::effects::Effects,
    pub mentions: Slots,
    pub writes: Slots,
    pub node_writes: Slots,
}

pub(super) type CountBounds = (i64, Option<i64>, Option<(usize, i64)>);

pub(super) enum Instruction {
    Eval(usize),
    Call(usize),
    Boolean {
        all: bool,
        children: Vec<usize>,
    },
    Not(usize),
    Search {
        axis: Axis,
        end: bool,
        field: Option<u16>,
        body: usize,
        stop: Option<usize>,
        bounds: Option<CountBounds>,
        filter: Option<WalkFilter>,
    },
    Binding {
        core: Box<Plan>,
        children: Vec<(usize, usize)>,
        occurrence: Option<usize>,
    },
}

pub(super) struct Function {
    pub entry: usize,
    /// None is a transient utility entry, which spends work but bypasses memo.
    pub memo: Option<usize>,
    pub effects: super::effects::Effects,
    pub mentions: Slots,
    pub writes: Slots,
}

pub(super) struct Program {
    pub share_search: bool,
    pub code: Vec<Node>,
    pub functions: Vec<Function>,
    pub regions: Vec<Plan>,
    pub entry: usize,
    pub mentions: Slots,
    pub writes: Slots,
    pub node_writes: Slots,
}

#[path = "finite_intern.rs"]
mod finite_intern;
#[cfg(test)]
#[path = "finite_ir_inventory.rs"]
mod finite_ir_inventory;
#[path = "finite_recursive.rs"]
mod finite_recursive;
#[path = "finite_usage.rs"]
mod finite_usage;

use super::Op;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Default)]
struct Builder {
    code: Vec<Node>,
    functions: Vec<Function>,
    regions: Vec<Plan>,
    calls: HashMap<(usize, Option<usize>), usize>,
    call_nodes: HashMap<usize, usize>,
    recursive_functions: HashMap<(usize, usize), usize>,
    share_recursion: bool,
    active: HashSet<usize>,
    shared_regions: HashMap<(u8, usize, usize), usize>,
    predicates: Vec<(super::super::ir::Set, usize)>,
    shared: HashMap<usize, usize>,
    uses: HashMap<usize, usize>,
}

fn component(plan: &Plan) -> Option<(u8, usize, usize)> {
    Some(match &plan.op {
        Op::PureRecursive(p) => (0, Rc::as_ptr(p) as usize, 0),
        Op::RecursiveSearch { component, .. } => (1, Rc::as_ptr(component) as usize, 0),
        Op::Chain(_, p) => (2, Rc::as_ptr(p) as usize, 0),
        Op::Downward(_, p) => (3, Rc::as_ptr(p) as usize, 0),
        Op::MixedRecursive { program, entry } => (4, Rc::as_ptr(program) as usize, *entry),
        Op::ParsedRecursive { program, entry } => (5, Rc::as_ptr(program) as usize, *entry),
        _ => return None,
    })
}

fn atomic_capture(plan: &Plan) -> bool {
    match &plan.op {
        Op::Shared(body) => atomic_capture(body),
        Op::Bind(_) | Op::CaptureBind(_) | Op::Pattern(..) => true,
        _ => false,
    }
}
/// Regions with no shared function boundary keep their existing physical SQL.
fn boundary(plan: &Plan, uses: &HashMap<usize, usize>) -> bool {
    if component(plan).is_some() {
        return true;
    }
    match &plan.op {
        Op::Charge(..) | Op::MemoPredicate(..) | Op::TransientCharge(_) => true,
        Op::Shared(body) => {
            (!atomic_capture(body)
                && uses.get(&(Rc::as_ptr(body) as usize)).copied().unwrap_or(0) > 1)
                || boundary(body, uses)
        }
        Op::All(v) | Op::Any(v) => v.iter().any(|p| boundary(p, uses)),
        Op::Not(p) | Op::FieldNeighbor { child: p, .. } | Op::FieldEnd { child: p, .. } => {
            boundary(p, uses)
        }
        Op::Search(_, _, p, s)
        | Op::Count {
            rule: p, stop: s, ..
        } => boundary(p, uses) || s.as_ref().is_some_and(|p| boundary(p, uses)),
        Op::Binding {
            core,
            children,
            occurrence,
            ..
        } => {
            boundary(core, uses)
                || children.iter().any(|(_, p)| boundary(p, uses))
                || occurrence.as_ref().is_some_and(|p| boundary(p, uses))
        }
        _ => false,
    }
}
impl Builder {
    fn region(&mut self, plan: &Plan) -> usize {
        if let Op::Predicate(set) = &plan.op
            && let Some((_, pc)) = self.predicates.iter().find(|(other, _)| other == set)
        {
            return *pc;
        }
        if let Some(key) = component(plan)
            && let Some(&pc) = self.shared_regions.get(&key)
        {
            return pc;
        }
        let region = self.regions.len();
        self.regions.push(plan.clone());
        let pc = self.node(plan, Instruction::Eval(region));
        if let Op::Predicate(set) = &plan.op {
            self.predicates.push((set.clone(), pc));
        }
        if let Some(key) = component(plan) {
            self.shared_regions.insert(key, pc);
        }
        pc
    }
    fn node(&mut self, plan: &Plan, op: Instruction) -> usize {
        if let Some(pc) = self
            .code
            .iter()
            .position(|node| finite_intern::same(node, plan, &op))
        {
            return pc;
        }
        let pc = self.code.len();
        self.code.push(Node {
            op,
            effects: plan.effects,
            mentions: plan.mentions.clone(),
            writes: plan.writes.clone(),
            node_writes: plan.node_writes.clone(),
        });
        pc
    }
    fn call(&mut self, plan: &Plan, body: &Rc<Plan>, memo: Option<usize>) -> Result<usize, String> {
        let pointer = Rc::as_ptr(body) as usize;
        let key = (pointer, memo);
        let function = if let Some(&id) = self.calls.get(&key) {
            if self.active.contains(&id) {
                return Err("finite control: cyclic ordinary utility body".into());
            }
            id
        } else {
            let id = self.functions.len();
            self.calls.insert(key, id);
            self.functions.push(Function {
                entry: usize::MAX,
                memo,
                effects: body.effects,
                mentions: body.mentions.clone(),
                writes: body.writes.clone(),
            });
            self.active.insert(id);
            let entry = self.plan(body)?;
            self.active.remove(&id);
            self.functions[id].entry = entry;
            id
        };
        if let Some(&pc) = self.call_nodes.get(&function) {
            return Ok(pc);
        }
        let pc = self.node(plan, Instruction::Call(function));
        self.call_nodes.insert(function, pc);
        Ok(pc)
    }
    fn plan(&mut self, plan: &Plan) -> Result<usize, String> {
        if let Op::Shared(body) = &plan.op {
            let pointer = Rc::as_ptr(body) as usize;
            if let Some(&pc) = self.shared.get(&pointer) {
                return Ok(pc);
            }
            let pc = self.plan(body)?;
            self.shared.insert(pointer, pc);
            return Ok(pc);
        }
        if self.share_recursion
            && let Op::RecursiveSearch { id, component } = &plan.op
            && finite_recursive::has_functions(&component.base, &mut HashSet::new())
        {
            return self.recursive(*id, component, plan);
        }
        if component(plan).is_some()
            || matches!(plan.op, Op::Predicate(_))
            || !boundary(plan, &self.uses)
        {
            return Ok(self.region(plan));
        }
        let op = match &plan.op {
            Op::Charge(id, body) | Op::MemoPredicate(id, body) => {
                return self.call(plan, body, Some(*id));
            }
            Op::TransientCharge(body) => return self.call(plan, body, None),
            Op::All(items) | Op::Any(items) => Instruction::Boolean {
                all: matches!(plan.op, Op::All(_)),
                children: items
                    .iter()
                    .map(|p| self.plan(p))
                    .collect::<Result<_, _>>()?,
            },
            Op::Not(p) => Instruction::Not(self.plan(p)?),
            Op::FieldNeighbor {
                inside,
                field,
                child,
            }
            | Op::FieldEnd {
                inside,
                field,
                child,
            } => Instruction::Search {
                axis: if *inside { Axis::Parent } else { Axis::Child },
                end: matches!(plan.op, Op::FieldEnd { .. }),
                field: Some(*field),
                body: self.plan(child)?,
                stop: None,
                bounds: None,
                filter: child.walk_filter.clone(),
            },
            Op::Search(axis, end, body, stop) => Instruction::Search {
                axis: *axis,
                end: *end,
                field: None,
                body: self.plan(body)?,
                stop: stop.as_ref().map(|p| self.plan(p)).transpose()?,
                bounds: None,
                filter: body.walk_filter.clone(),
            },
            Op::Count {
                axis,
                end,
                rule,
                stop,
                min,
                max,
                binding,
            } => Instruction::Search {
                axis: *axis,
                end: *end,
                field: None,
                body: self.plan(rule)?,
                stop: stop.as_ref().map(|p| self.plan(p)).transpose()?,
                bounds: Some((*min, *max, *binding)),
                filter: rule.walk_filter.clone(),
            },
            Op::Binding {
                core,
                children,
                occurrence,
                ..
            } => Instruction::Binding {
                core: core.clone(),
                children: children
                    .iter()
                    .map(|(s, p)| Ok((*s, self.plan(p)?)))
                    .collect::<Result<_, String>>()?,
                occurrence: occurrence.as_ref().map(|p| self.plan(p)).transpose()?,
            },
            _ => return Err("finite control: unsupported function-bearing boundary".into()),
        };
        Ok(self.node(plan, op))
    }
}

pub(super) fn lower(plan: &Plan) -> Result<Program, String> {
    lower_with_recursion(plan, true)
}
pub(super) fn lower_with_recursion(plan: &Plan, share_recursion: bool) -> Result<Program, String> {
    let mut builder = Builder {
        share_recursion,
        uses: finite_usage::count(plan),
        ..Builder::default()
    };
    let entry = builder.plan(plan)?;
    if builder.code.len() > 256 {
        return Err("finite control: distinct control operators exceed 256".into());
    }
    Ok(Program {
        share_search: share_recursion,
        code: builder.code,
        functions: builder.functions,
        regions: builder.regions,
        entry,
        mentions: plan.mentions.clone(),
        writes: plan.writes.clone(),
        node_writes: plan.node_writes.clone(),
    })
}
