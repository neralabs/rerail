//! The lowering's whole-program tail on the version (decision 0281's path, step 7b): the passes `whole()` runs once
//! the program is finished, each one for one with its tree form in `rr::lower`, which a debug build runs beside it
//! (`rr::link_seeded_version`).

use crate::code_rule_dsl::logic::plan::body::{Body, VarSet, lit_vars};
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{Lit as L, RuleId, Slot};
use std::collections::{BTreeMap, BTreeSet};

/// One round of `widen_cyclic_contexts` (see there): each context relation's rule in a component a negation or an
/// aggregate closes a cycle in, its literals of that component dropped, the tests the rest binds kept, and a node
/// column left unbound ranging over `node`. `changed` says whether the round widened one; the caller runs rounds
/// until none does, as the tree's loop does (16 at most).
#[derive(Default)]
pub struct WidenCyclicContexts {
    pub changed: bool,
}

/// A widened rule: its kept literals by node, in order, and the head variables then ranged over `node`.
struct Widened {
    lits: Vec<u32>,
    nodes: Vec<Slot>,
}

impl ProgramPass for WidenCyclicContexts {
    const NAME: &'static str = "widen the cyclic contexts";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let ctx_prefix = format!("{}ctx", crate::code_rule_dsl::logic::front::RESERVED);
        let about = cx.get::<crate::code_rule_dsl::logic::program_image::About>()?;
        let errors = crate::code_rule_dsl::logic::program_image::analyses::unstratified(v, &about);
        let sccs: Vec<BTreeSet<String>> = errors.iter().filter_map(|e| e.unstratified().map(|(scc, _)| scc.clone())).collect();
        if sccs.is_empty() {
            return Ok(());
        }
        let header = v.header(&Default::default())?;
        let node_declared = header.decls.contains_key("node");
        let mut widened: BTreeMap<RuleId, Widened> = BTreeMap::new();
        for scc in &sccs {
            for rule in v.rules() {
                let head = v.rel_name(rule.head().rel());
                if !head.starts_with(&ctx_prefix) || !scc.contains(head) {
                    continue;
                }
                let in_cycle = |l: &L<'_>| match l {
                    L::Atom(a) | L::Agg { atom: a, .. } => scc.contains(v.rel_name(a.rel())),
                    _ => false,
                };
                if !rule.body().any(|l| in_cycle(&l.view())) {
                    continue;
                }
                let kept: Vec<_> = rule.body().filter(|l| !in_cycle(&l.view())).collect();
                // What the positive atoms left bind; then the literals that only test.
                let binds = |l: &L<'_>| matches!(l, L::Atom(a) if !a.neg()) || matches!(l, L::Nearest { .. } | L::Ast { .. });
                let mut bound = VarSet::new();
                for l in kept.iter().filter(|l| binds(&l.view())) {
                    bound.extend(&lit_vars(*l));
                }
                let mut lits: Vec<u32> = kept.iter().filter(|l| binds(&l.view())).map(|l| l.node()).collect();
                let mut rest: Vec<_> = kept.into_iter().filter(|l| !binds(&l.view())).collect();
                let body = Body::of(v, rule);
                loop {
                    let before = rest.len();
                    rest.retain(|l| {
                        let ready = match l.view() {
                            L::Cmp { l: a, op, r } => body.cmp_binds(a, op, r, &bound),
                            // An aggregate is left out: it only narrows, and its body's variables are its own.
                            L::Agg { .. } => None,
                            _ => lit_vars(*l).is_subset(&bound).then(VarSet::new),
                        };
                        match ready {
                            Some(new) => {
                                bound.extend(&new);
                                lits.push(l.node());
                                false
                            }
                            None => true,
                        }
                    });
                    if rest.len() == before {
                        break;
                    }
                }
                // The head's variables by name (the tree's `BTreeSet<String>`), each with its column's type.
                let cols = header.decls.get(head);
                let mut types: BTreeMap<Slot, &str> = BTreeMap::new();
                let mut head_vars: BTreeMap<String, Slot> = BTreeMap::new();
                for (k, t) in rule.head().args().enumerate() {
                    if let Some(s) = t.var() {
                        if let Some((_, ty)) = cols.and_then(|c| c.get(k)) {
                            types.insert(s, ty.as_ref());
                        }
                    }
                    let mut vs = VarSet::new();
                    crate::code_rule_dsl::logic::plan::body::term_vars(t, &mut vs);
                    for s in vs.iter() {
                        head_vars.insert(v.text(rule.var(s)).to_owned(), s);
                    }
                }
                let mut nodes = Vec::new();
                let mut ok = true;
                for s in head_vars.into_values() {
                    if bound.contains(s) {
                        continue;
                    }
                    if types.get(&s).is_some_and(|t| *t == "node") && node_declared {
                        nodes.push(s);
                        bound.insert(s);
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    widened.insert(rule.id(), Widened { lits, nodes });
                }
            }
        }
        if widened.is_empty() {
            return Ok(());
        }
        self.changed = true;
        let input = std::sync::Arc::clone(w.input());
        let b = w.derive()?;
        for rule in input.rules() {
            let Some(wd) = widened.get(&rule.id()) else {
                b.keep(&input, rule);
                continue;
            };
            let mut rb = b.rule().source(rule.id()).head_from(&input, rule, rule.head());
            for &n in &wd.lits {
                let l = rule.body().find(|l| l.node() == n).expect("a literal of the rule");
                rb = rb.lit(&input, rule, l);
            }
            for &s in &wd.nodes {
                rb = rb.atom("node", &[T::Wild, T::VarAtom(rule.var(s)), T::Wild, T::Wild, T::Wild]);
            }
            rb.finish()?;
        }
        Ok(())
    }
}

/// A body is a conjunction: a literal twice is the literal once (`whole()`'s "a literal twice"), its first kept. Two
/// literals of one rule are equal where they print alike: the rule's variables have one name each.
pub struct LiteralsOnce;

impl ProgramPass for LiteralsOnce {
    const NAME: &'static str = "each literal of a body once";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let mut once: BTreeMap<RuleId, Vec<u32>> = BTreeMap::new();
        for rule in v.rules() {
            let shown: Vec<(u32, String)> = rule.body().map(|l| (l.node(), v.show_lit(rule, l))).collect();
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let kept: Vec<u32> = shown.iter().filter(|(_, s)| seen.insert(s.as_str())).map(|(n, _)| *n).collect();
            if kept.len() < shown.len() {
                once.insert(rule.id(), kept);
            }
        }
        if once.is_empty() {
            return Ok(());
        }
        let input = std::sync::Arc::clone(w.input());
        let b = w.derive()?;
        for rule in input.rules() {
            let Some(kept) = once.get(&rule.id()) else {
                b.keep(&input, rule);
                continue;
            };
            let mut rb = b.rule().source(rule.id()).head_from(&input, rule, rule.head());
            for &n in kept {
                let l = rule.body().find(|l| l.node() == n).expect("a literal of the rule");
                rb = rb.lit(&input, rule, l);
            }
            rb.finish()?;
        }
        Ok(())
    }
}

/// A rule whose body is its own head derives nothing (`r(x) :- dfs_first_memo(x, …)` printed, whose construct lowers
/// to the relation itself): dropped, as `whole()` drops it.
pub struct DropSelfRules;

impl ProgramPass for DropSelfRules {
    const NAME: &'static str = "drop the rules that are their own body";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let args = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| a.args().map(|t| v.show_term(r, t)).collect::<Vec<_>>();
        let own = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| {
            let mut body = r.body();
            match (body.next().map(|l| l.view()), body.next()) {
                (Some(L::Atom(a)), None) => !a.neg() && a.rel() == r.head().rel() && args(r, a) == args(r, r.head()),
                _ => false,
            }
        };
        if v.rules().any(own) {
            w.keep(|r| !own(r))?;
        }
        Ok(())
    }
}

/// A relation's facts are a set too (`whole()`'s last set): each row once, its first kept.
pub struct FactsOnce;

impl ProgramPass for FactsOnce {
    const NAME: &'static str = "each fact once";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let was = v.header(&Default::default())?;
        let mut head = was.clone();
        for rows in head.facts.values_mut() {
            let mut kept: Vec<crate::code_rule_dsl::logic::front::List<crate::code_rule_dsl::logic::front::Value>> = Vec::with_capacity(rows.len());
            for row in rows.drain(..) {
                if !kept.contains(&row) {
                    kept.push(row);
                }
            }
            *rows = kept;
        }
        if head.facts != was.facts {
            w.on()?.set_header(&was, &head);
        }
        Ok(())
    }
}

/// `Lowerer::unit`'s rule over the program (`lower::unit`): each rule that reads no relation reads `rr_unit(0)`
/// first, and `rr_unit` is declared with its one fact. A matcher atom or a walk reads the tree itself; an aggregate
/// reads its atom.
pub struct Unit;

impl ProgramPass for Unit {
    const NAME: &'static str = "a rule reading no relation reads the unit";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        const UNIT: &str = "rr_unit";
        let v = cx.v();
        let reads = |l: L<'_>| matches!(l, L::Atom(a) if !a.neg()) || matches!(l, L::Ast { .. } | L::Nearest { .. } | L::Agg { .. });
        let lone: BTreeSet<RuleId> = v.rules().filter(|r| !r.body().any(|l| reads(l.view()))).map(|r| r.id()).collect();
        if lone.is_empty() {
            return Ok(());
        }
        let was = v.header(&Default::default())?;
        let mut head = was.clone();
        head.decls.insert(UNIT.to_owned().into(), [("x".into(), "int".into())].into());
        head.facts.insert(UNIT.to_owned(), vec![[crate::code_rule_dsl::logic::front::Value::Int(0)].into()]);
        let input = std::sync::Arc::clone(w.input());
        let b = w.derive()?;
        b.set_header(&was, &head);
        for rule in input.rules() {
            if !lone.contains(&rule.id()) {
                b.keep(&input, rule);
                continue;
            }
            let mut rb = b.rule().source(rule.id()).head_from(&input, rule, rule.head()).atom(UNIT, &[T::Int(0)]);
            for l in rule.body() {
                rb = rb.lit(&input, rule, l);
            }
            rb.finish()?;
        }
        Ok(())
    }
}
