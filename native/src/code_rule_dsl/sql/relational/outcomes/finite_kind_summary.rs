//! Complete metadata-proved utility bodies. Unknown paths retain their scheduler.
//! A proof carries logical cost, peak, and nested memo completions together.
use super::lower::recursive_search_lower::Arm;
use super::{
    Op, Plan,
    finite_lower::{Instruction, Program},
    pure_kind_summary,
};
use std::collections::{BTreeSet, HashSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Completion {
    pub utility: usize,
    pub truth: bool,
    pub cost: u64,
    pub peak: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Answer {
    pub truth: bool,
    pub cost: u64,
    pub peak: u32,
    pub completions: Vec<Completion>,
}
impl Answer {
    fn atom(truth: bool) -> Self {
        Self {
            truth,
            cost: 0,
            peak: 0,
            completions: vec![],
        }
    }
    fn append(&mut self, next: Self) -> Option<()> {
        self.truth = next.truth;
        self.cost = self.cost.checked_add(next.cost)?;
        self.peak = self.peak.max(next.peak);
        self.completions.extend(next.completions);
        (self.cost <= 65536 && self.completions.len() <= 256).then_some(())
    }
    fn enter(mut self, memo: Option<usize>) -> Option<Self> {
        self.cost += 1;
        self.peak += 1;
        if let Some(utility) = memo {
            self.completions.push(Completion {
                utility,
                truth: self.truth,
                cost: self.cost,
                peak: self.peak,
            });
        }
        (self.peak <= 256 && self.cost <= 65536 && self.completions.len() <= 256).then_some(self)
    }
}
struct Eval<'a> {
    program: &'a Program,
    active: HashSet<usize>,
    remaining: usize,
}
impl Eval<'_> {
    fn spend(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }
    fn code(&mut self, pc: usize, kind: Option<u16>) -> Option<Answer> {
        self.spend()?;
        if !self.active.insert(pc) {
            return None;
        }
        let result = (|| match &self.program.code[pc].op {
            Instruction::Eval(region) => self.plan(&self.program.regions[*region], kind),
            Instruction::Call(f) => {
                let f = &self.program.functions[*f];
                self.code(f.entry, kind)?.enter(f.memo)
            }
            Instruction::Not(child) => {
                let mut a = self.code(*child, kind)?;
                a.truth = !a.truth;
                Some(a)
            }
            Instruction::Boolean { all, children } => {
                let mut a = Answer::atom(*all);
                for c in children {
                    a.append(self.code(*c, kind)?)?;
                    if a.truth != *all {
                        break;
                    }
                }
                Some(a)
            }
            _ => None,
        })();
        self.active.remove(&pc);
        result
    }
    fn plan(&mut self, p: &Plan, kind: Option<u16>) -> Option<Answer> {
        self.spend()?;
        if !p.mentions.is_empty() || !p.writes.is_empty() || !p.node_writes.is_empty() {
            return None;
        }
        match &p.op {
            Op::Predicate(set) => Some(Answer::atom(pure_kind_summary::predicate(set, kind)?)),
            Op::Shared(body) => self.plan(body, kind),
            Op::Charge(id, body) => self.plan(body, kind)?.enter(Some(*id)),
            Op::TransientCharge(body) => self.plan(body, kind)?.enter(None),
            Op::Not(body) => {
                let mut a = self.plan(body, kind)?;
                a.truth = !a.truth;
                Some(a)
            }
            Op::All(children) | Op::Any(children) => {
                let all = matches!(p.op, Op::All(_));
                let mut a = Answer::atom(all);
                for child in children {
                    a.append(self.plan(child, kind)?)?;
                    if a.truth != all {
                        break;
                    }
                }
                Some(a)
            }
            Op::RecursiveSearch { id, component } => {
                let bases = if component.ordered_bases {
                    let Op::Any(v) = &component.base.op else {
                        return None;
                    };
                    v.iter().collect::<Vec<_>>()
                } else {
                    vec![&component.base]
                };
                let mut a = Answer::atom(false);
                for arm in &component.arms {
                    match arm {
                        Arm::Base(i) => a.append(self.plan(bases.get(*i)?, kind)?)?,
                        Arm::Search { guard, .. } => {
                            if pure_kind_summary::predicate(guard, kind)? {
                                return None;
                            }
                            a.truth = false;
                        }
                    }
                    if a.truth {
                        break;
                    }
                }
                a.enter((component.memo && !component.bypass).then_some(2_000_000 + id))
            }
            // In particular Value, unknown, structural search, caches, and captures
            // cannot enter the proof. A prior false guard may keep them unreached.
            _ => None,
        }
    }
}
fn plan_kinds(p: &Plan, seen: &mut HashSet<usize>, out: &mut BTreeSet<u16>) {
    if !seen.insert(std::ptr::from_ref(p) as usize) {
        return;
    }
    match &p.op {
        Op::Predicate(s) => pure_kind_summary::kinds(s, out),
        Op::Shared(p) | Op::Charge(_, p) | Op::TransientCharge(p) => plan_kinds(p, seen, out),
        Op::Not(p) => plan_kinds(p, seen, out),
        Op::All(v) | Op::Any(v) => {
            for p in v {
                plan_kinds(p, seen, out)
            }
        }
        Op::RecursiveSearch { component, .. } => {
            plan_kinds(&component.base, seen, out);
            for arm in &component.arms {
                if let Arm::Search { guard, .. } = arm {
                    pure_kind_summary::kinds(guard, out)
                }
            }
        }
        _ => {}
    }
}
pub(super) struct Stratum {
    pub function: usize,
    pub kinds: Vec<Option<u16>>,
    pub answer: Answer,
}
pub(super) struct Summaries {
    pub palette: Vec<u16>,
    pub strata: Vec<Stratum>,
    /// Original search sites whose other candidate kinds are silent false.
    pub search_candidates: std::collections::BTreeMap<usize, Vec<u16>>,
}
pub(super) fn compile(p: &Program) -> Summaries {
    let mut palette = BTreeSet::new();
    let mut seen = HashSet::new();
    for region in &p.regions {
        plan_kinds(region, &mut seen, &mut palette);
    }
    let palette = palette.into_iter().collect::<Vec<_>>();
    let mut strata: Vec<Stratum> = vec![];
    let mut remaining = 65536;
    // Restrict this adapter to capture-free programs. Failure and unknown paths
    // retain original bodies, so successful proofs do not erase failure followers.
    if p.mentions.is_empty()
        && p.writes.is_empty()
        && p.code
            .iter()
            .all(|n| n.mentions.is_empty() && n.writes.is_empty() && n.node_writes.is_empty())
    {
        for (function, f) in p.functions.iter().enumerate() {
            for kind in palette
                .iter()
                .copied()
                .map(Some)
                .chain(std::iter::once(None))
            {
                let mut eval = Eval {
                    program: p,
                    active: HashSet::new(),
                    remaining,
                };
                let answer = eval.code(f.entry, kind);
                remaining = eval.remaining;
                let Some(answer) = answer else {
                    continue;
                };
                // One entry cannot justify a separate dispatch and memo reduction.
                if answer.cost < 2 {
                    continue;
                }
                if let Some(s) = strata
                    .iter_mut()
                    .find(|s| s.function == function && s.answer == answer)
                {
                    s.kinds.push(kind)
                } else {
                    strata.push(Stratum {
                        function,
                        kinds: vec![kind],
                        answer,
                    });
                }
            }
        }
    }
    let mut search_candidates = std::collections::BTreeMap::new();
    for (site, node) in p.code.iter().enumerate() {
        let Instruction::Search {
            axis: super::Axis::Child,
            field: None,
            stop: None,
            bounds: None,
            body,
            ..
        } = node.op
        else {
            continue;
        };
        let mut silent_false = |kind| {
            let mut eval = Eval {
                program: p,
                active: HashSet::new(),
                remaining,
            };
            let answer = eval.code(body, kind);
            remaining = eval.remaining;
            answer
                .is_some_and(|a| !a.truth && a.cost == 0 && a.peak == 0 && a.completions.is_empty())
        };
        if !silent_false(None) {
            continue;
        }
        let possible = palette
            .iter()
            .copied()
            .filter(|kind| !silent_false(Some(*kind)))
            .collect();
        search_candidates.insert(site, possible);
    }
    Summaries {
        palette,
        strata,
        search_candidates,
    }
}
