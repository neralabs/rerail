//! Bounded static continuation graph for capture-free finite composition.
//! Utility calls remain atomic: their original function identities are not interned away.
use super::finite_lower::{Instruction, Program};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    Site(usize),
    Return(i8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Routes(pub [Target; 4]);
impl Routes {
    pub(super) const RETURN: Self = Self([
        Target::Return(0),
        Target::Return(1),
        Target::Return(2),
        Target::Return(-1),
    ]);
}
#[derive(Clone, Debug)]
pub(super) struct Site {
    pub source: usize,
    pub routes: Routes,
}
pub(super) struct Cfg {
    pub sites: Vec<Site>,
    pub entry: Target,
    pub functions: Vec<Target>,
    pub entries: HashMap<usize, Target>,
}
struct Builder<'a> {
    program: &'a Program,
    sites: Vec<Site>,
    intern: HashMap<(usize, Routes), Target>,
    visits: usize,
}
impl Builder<'_> {
    fn lower(&mut self, node: usize, r: Routes) -> Option<Target> {
        self.visits += 1;
        if self.visits > 65536 {
            return None;
        }
        if let Some(target) = self.intern.get(&(node, r)) {
            return Some(*target);
        }
        let program = self.program;
        let result = match &program.code[node].op {
            Instruction::Not(child) => {
                self.lower(*child, Routes([r.0[1], r.0[0], r.0[2], r.0[3]]))?
            }
            Instruction::Boolean { all, children }
                if *all
                    || children
                        .iter()
                        .skip(1)
                        .all(|child| !fault_observable(self.program, *child)) =>
            {
                let mut clean = if *all { r.0[1] } else { r.0[0] };
                let mut uncertain = r.0[2];
                for child in children.iter().rev() {
                    let u = if *all {
                        Routes([r.0[0], uncertain, uncertain, r.0[3]])
                    } else {
                        Routes([uncertain, r.0[1], uncertain, r.0[3]])
                    };
                    let c = if *all {
                        Routes([r.0[0], clean, uncertain, r.0[3]])
                    } else {
                        Routes([clean, r.0[1], uncertain, r.0[3]])
                    };
                    let next_uncertain = self.lower(*child, u)?;
                    clean = self.lower(*child, c)?;
                    uncertain = next_uncertain;
                }
                clean
            }
            _ => {
                if self.sites.len() >= 256 {
                    return None;
                }
                let id = self.sites.len();
                self.sites.push(Site {
                    source: node,
                    routes: r,
                });
                Target::Site(id)
            }
        };
        self.intern.insert((node, r), result);
        Some(result)
    }
}

fn fault_observable(p: &Program, node: usize) -> bool {
    match &p.code[node].op {
        Instruction::Call(_) | Instruction::Search { .. } | Instruction::Binding { .. } => true,
        Instruction::Boolean { children, .. } => {
            children.iter().any(|child| fault_observable(p, *child))
        }
        Instruction::Not(child) => fault_observable(p, *child),
        Instruction::Eval(_) => {
            let e = p.code[node].effects;
            e.uses_memo || e.file_memo || e.cache_fills || e.fallible || e.needs_peak
        }
    }
}

pub(super) fn compile(program: &Program) -> Option<Cfg> {
    if !program.writes.is_empty()
        || program
            .code
            .iter()
            .any(|n| !n.writes.is_empty() || !n.node_writes.is_empty())
    {
        return None;
    }
    let mut b = Builder {
        program,
        sites: vec![],
        intern: HashMap::new(),
        visits: 0,
    };
    let entry = b.lower(program.entry, Routes::RETURN)?;
    let functions = program
        .functions
        .iter()
        .map(|f| b.lower(f.entry, Routes::RETURN))
        .collect::<Option<Vec<_>>>()?;
    // Search/Binding children execute in their own existing structural transaction.
    // Lower those entries with identity return routes before materializing the adapter.
    let mut cursor = 0;
    while cursor < b.sites.len() {
        let source = b.sites[cursor].source;
        match &program.code[source].op {
            Instruction::Boolean { children, .. } => {
                for child in children {
                    b.lower(*child, Routes::RETURN)?;
                }
            }
            Instruction::Search { body, stop, .. } => {
                b.lower(*body, Routes::RETURN)?;
                if let Some(stop) = stop {
                    b.lower(*stop, Routes::RETURN)?;
                }
            }
            Instruction::Binding {
                children,
                occurrence,
                ..
            } => {
                if let Some(occurrence) = occurrence {
                    b.lower(*occurrence, Routes::RETURN)?;
                }
                for (_, child) in children {
                    b.lower(*child, Routes::RETURN)?;
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    if !matches!(entry, Target::Site(_))
        || functions
            .iter()
            .any(|target| !matches!(target, Target::Site(_)))
    {
        return None;
    }
    let removes_boolean = program.code.iter().enumerate().any(|(i, node)| {
        matches!(node.op, Instruction::Boolean { .. } | Instruction::Not(_))
            && !b.sites.iter().any(|site| site.source == i)
    });
    if !removes_boolean {
        return None;
    }
    // CPS construction also creates unknown-state suffixes. Remove those that
    // cannot be reached from the actual entry or any structural child entry.
    fn may_unknown(p: &Program, n: usize, active: &mut std::collections::HashSet<usize>) -> bool {
        if !active.insert(n) {
            return true;
        }
        let result = match &p.code[n].op {
            Instruction::Eval(r) => p.regions.get(*r).is_none_or(|r| {
                !matches!(
                    r.op,
                    super::Op::Predicate(_)
                        | super::Op::PureRecursive(_)
                        | super::Op::Chain(..)
                        | super::Op::Downward(..)
                )
            }),
            Instruction::Call(f) => may_unknown(p, p.functions[*f].entry, active),
            Instruction::Not(c) => may_unknown(p, *c, active),
            Instruction::Boolean { children, .. } => {
                children.iter().any(|c| may_unknown(p, *c, active))
            }
            Instruction::Search { body, stop, .. } => {
                may_unknown(p, *body, active) || stop.is_some_and(|s| may_unknown(p, s, active))
            }
            _ => true,
        };
        active.remove(&n);
        result
    }
    let identities = b
        .intern
        .iter()
        .filter(|((_, r), _)| *r == Routes::RETURN)
        .map(|((s, _), t)| (*s, *t))
        .collect::<HashMap<_, _>>();
    let mut pending = vec![entry];
    pending.extend(functions.iter().copied());
    let mut live = std::collections::BTreeSet::new();
    while let Some(target) = pending.pop() {
        let Target::Site(i) = target else { continue };
        if !live.insert(i) {
            continue;
        }
        let site = &b.sites[i];
        let unknown = may_unknown(program, site.source, &mut std::collections::HashSet::new());
        pending.extend(
            site.routes
                .0
                .iter()
                .enumerate()
                .filter_map(|(v, t)| (v != 2 || unknown).then_some(*t))
                .filter(|t| matches!(t, Target::Site(_))),
        );
        let children = match &program.code[site.source].op {
            Instruction::Boolean { children, .. } => children.clone(),
            Instruction::Search { body, stop, .. } => std::iter::once(*body).chain(*stop).collect(),
            Instruction::Binding {
                children,
                occurrence,
                ..
            } => children
                .iter()
                .map(|(_, c)| *c)
                .chain(*occurrence)
                .collect(),
            _ => vec![],
        };
        pending.extend(children.iter().filter_map(|c| identities.get(c)).copied());
    }
    let mapping = live
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new))
        .collect::<HashMap<_, _>>();
    let remap = |target: Target| match target {
        Target::Site(i) => mapping
            .get(&i)
            .map_or(Target::Return(2), |i| Target::Site(*i)),
        other => other,
    };
    let sites = live
        .iter()
        .map(|i| Site {
            source: b.sites[*i].source,
            routes: Routes(b.sites[*i].routes.0.map(remap)),
        })
        .collect();
    Some(Cfg {
        entries: identities
            .into_iter()
            .filter(|(_, t)| matches!(t,Target::Site(i) if live.contains(i)))
            .map(|(s, t)| (s, remap(t)))
            .collect(),
        sites,
        entry: remap(entry),
        functions: functions.into_iter().map(remap).collect(),
    })
}

#[cfg(test)]
#[path = "finite_cfg_tests.rs"]
mod tests;
