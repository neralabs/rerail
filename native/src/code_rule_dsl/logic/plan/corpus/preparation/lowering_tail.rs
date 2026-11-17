//! The lowering's whole-program tail on the version (decision 0281's path, step 7b): `front::finish`'s rewrites and
//! the passes `whole()` runs once the program is finished, each one for one with its tree form in `front` and
//! `rr::lower`, which a debug build runs beside it (`rr::link_seeded_version`).

use crate::code_rule_dsl::logic::plan::body::{Body, VarSet, lit_vars};
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{Lit as L, RuleId, Slot, TermRef, Version};
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

/// `aggregate_demand::optimize` on the version (see there): each private aggregate helper (`rr_agg…`) restricted
/// to the keys its consumers ask (`rr_demand_agg_<helper>`, read first by its rules and made from each consumer's
/// guards), placed one helper at a time and refused where it makes the program unstratifiable; then a `count` of
/// zero read once made an anti-join. The tree's order is kept: a helper's rules gain their demand where they stand,
/// and the demand rules follow the program's.
///
/// The program is taken to be declared: `lower::link_seeded_front` checks it before this runs, and the tree's pass
/// places nothing in a program that is not (which `link_seeded` then refuses).
#[derive(Default)]
pub struct AggregateDemand;

/// A literal of a rule as the pass sees it: one of its own, or the demand placed first in a helper's rule.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Own(RuleId, u32),
    Placed(RuleId),
}

/// A demand rule: its helper's demand, made from consumer `src`'s aggregate at `agg`, its body `body`.
struct DemandRule {
    demand: String,
    src: RuleId,
    agg: u32,
    positions: Vec<usize>,
    body: Vec<Item>,
}

/// What `optimize` has placed so far.
#[derive(Default)]
struct Placement {
    /// A helper's rule: the demand it reads first, over its head's columns `positions`.
    prefix: BTreeMap<RuleId, (String, Vec<usize>)>,
    demands: Vec<DemandRule>,
    decls: Vec<(String, Vec<(String, String)>)>,
}

impl AggregateDemand {
    /// Item `i`'s relation, whether it is a positive atom (`Some(false)`), a negated one or an aggregate's
    /// (`Some(true)`, an aggregate's atom counted as read), or neither (`None`).
    fn rel<'v>(v: &'v Version, p: &'v Placement, i: Item) -> Option<(&'v str, bool, bool)> {
        match i {
            Item::Placed(r) => Some((p.prefix[&r].0.as_str(), true, false)),
            Item::Own(r, n) => match lit_at(v, r, n).view() {
                L::Atom(a) => Some((v.rel_name(a.rel()), !a.neg(), false)),
                L::Agg { atom, .. } => Some((v.rel_name(atom.rel()), false, true)),
                _ => None,
            },
        }
    }
    fn vars(v: &Version, p: &Placement, i: Item) -> VarSet {
        match i {
            Item::Placed(r) => {
                let rule = v.rule(r).expect("a rule of the input");
                let mut out = VarSet::new();
                for (k, t) in rule.head().args().enumerate() {
                    if p.prefix[&r].1.contains(&k) {
                        crate::code_rule_dsl::logic::plan::body::term_vars(t, &mut out);
                    }
                }
                out
            }
            Item::Own(r, n) => lit_vars(lit_at(v, r, n)),
        }
    }
}

fn lit_at(v: &Version, r: RuleId, n: u32) -> crate::code_rule_dsl::logic::program_image::LitRef<'_> {
    v.rule(r).expect("a rule of the input").body().find(|l| l.node() == n).expect("a literal of the rule")
}

/// `aggregate_demand::guards` over rule `r`'s items: its safe positive guards joined to the keys, then the
/// comparisons they let run.
#[allow(clippy::too_many_arguments)]
fn guards(v: &Version, p: &Placement, r: RuleId, items: &[Item], helper: &str, keys: &VarSet, deps: &BTreeMap<String, BTreeSet<String>>, reach: &crate::code_rule_dsl::logic::rr::aggregate_demand::Reach) -> Option<Vec<Item>> {
    let rule = v.rule(r).expect("a rule of the input");
    let head = v.rel_name(rule.head().rel());
    let safe: Vec<(Item, VarSet)> = items
        .iter()
        .filter(|&&i| match i {
            Item::Own(_, n) if matches!(lit_at(v, r, n).view(), L::Nearest { .. }) => true,
            i => matches!(AggregateDemand::rel(v, p, i), Some((rel, true, false)) if !reach.reaches(deps, rel, helper) && !reach.reaches(deps, rel, head)),
        })
        .map(|&i| (i, AggregateDemand::vars(v, p, i)))
        .collect();
    let cmps: Vec<(usize, u32, VarSet)> = items
        .iter()
        .enumerate()
        .filter_map(|(k, &i)| match i {
            Item::Own(_, n) if matches!(lit_at(v, r, n).view(), L::Cmp { .. }) => Some((k, n, lit_vars(lit_at(v, r, n)))),
            _ => None,
        })
        .collect();
    let body = Body::of(v, rule);
    let mut connected = keys.clone();
    let mut grounded = VarSet::new();
    let mut bound = VarSet::new();
    let mut joined = vec![false; safe.len()];
    let mut added: BTreeSet<usize> = BTreeSet::new();
    let mut compared: Vec<Item> = Vec::new();
    loop {
        let before = (connected.len(), bound.len(), added.len());
        for (k, (i, vars)) in safe.iter().enumerate() {
            if joined[k] || vars.is_disjoint(&connected) {
                continue;
            }
            joined[k] = true;
            connected.extend(vars);
            if AggregateDemand::rel(v, p, *i).is_some() {
                grounded.extend(vars);
                bound.extend(vars);
            }
        }
        // A nearest binds its node once its subject is an atom's.
        for ((i, _), _) in safe.iter().zip(&joined).filter(|(_, j)| **j) {
            if let Item::Own(_, n) = i
                && let L::Nearest { n: subject, a, .. } = lit_at(v, r, *n).view()
                && grounded.contains(subject)
            {
                bound.insert(a);
            }
        }
        for (k, n, vars) in &cmps {
            if added.contains(k) {
                continue;
            }
            if let L::Cmp { l, op, r: right } = lit_at(v, r, *n).view()
                && let Some(more) = body.cmp_binds(l, op, right, &bound)
                && (vars.is_empty() || !vars.is_disjoint(&connected))
            {
                bound.extend(&more);
                connected.extend(vars);
                compared.push(Item::Own(r, *n));
                added.insert(*k);
            }
        }
        if (connected.len(), bound.len(), added.len()) == before {
            break;
        }
    }
    let mut out: Vec<Item> = safe.iter().zip(&joined).filter(|(_, j)| **j).map(|((i, _), _)| *i).collect();
    for i in &out {
        if let Item::Own(_, n) = i
            && let L::Nearest { n: subject, .. } = lit_at(v, r, *n).view()
            && !grounded.contains(subject)
        {
            return None;
        }
    }
    out.extend(compared);
    (keys.is_subset(&bound) && !out.is_empty()).then_some(out)
}

/// Writes rule `r` of `input` as placed: its demand first, where it has one, its count-of-zero aggregates `anti`
/// made anti-joins and their tests `gone` left out.
fn write_placed(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, input: &Version, p: &Placement, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, anti: &BTreeSet<u32>, gone: &BTreeSet<u32>) -> Result<(), String> {
    use crate::code_rule_dsl::logic::program_image::build::t_of;
    let mut rb = b.rule().source(r.id()).head_from(input, r, r.head());
    if let Some((demand, positions)) = p.prefix.get(&r.id()) {
        let args: Vec<T<'_>> = r.head().args().enumerate().filter(|(k, _)| positions.contains(k)).map(|(_, t)| t_of(input, r, t)).collect();
        rb = rb.atom(demand.as_str(), &args);
    }
    for l in r.body() {
        if gone.contains(&l.node()) {
            continue;
        }
        rb = match (anti.contains(&l.node()), l.view()) {
            (true, L::Agg { atom, .. }) => {
                let args: Vec<T<'_>> = atom.args().map(|t| t_of(input, r, t)).collect();
                rb.atom_with(input.rel_name(atom.rel()), &args, true, atom.gated())
            }
            _ => rb.lit(input, r, l),
        };
    }
    rb.finish().map(drop)
}

/// Writes demand rule `d`: its head over its consumer's aggregate's keys, its body its guards.
fn write_demand(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, input: &Version, p: &Placement, d: &DemandRule) -> Result<(), String> {
    use crate::code_rule_dsl::logic::program_image::build::t_of;
    let src = input.rule(d.src).expect("a rule of the input");
    let L::Agg { atom, .. } = lit_at(input, d.src, d.agg).view() else { return Err("a demand's aggregate".into()) };
    let args: Vec<T<'_>> = atom.args().enumerate().filter(|(k, _)| d.positions.contains(k)).map(|(_, t)| t_of(input, src, t)).collect();
    let mut rb = b.rule().source(d.src).head(d.demand.as_str(), &args);
    for &i in &d.body {
        rb = match i {
            Item::Own(r, n) => rb.lit(input, input.rule(r).expect("a rule"), lit_at(input, r, n)),
            Item::Placed(r) => {
                let rule = input.rule(r).expect("a rule");
                let (demand, positions) = &p.prefix[&r];
                let args: Vec<T<'_>> = rule.head().args().enumerate().filter(|(k, _)| positions.contains(k)).map(|(_, t)| t_of(input, rule, t)).collect();
                rb.atom(demand.as_str(), &args)
            }
        };
    }
    rb.finish().map(drop)
}

impl ProgramPass for AggregateDemand {
    const NAME: &'static str = "aggregate demand";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::program_image::analyses;
        use crate::code_rule_dsl::logic::rr::aggregate_demand::{Reach, depends, full_checks};
        let input = std::sync::Arc::clone(w.input());
        let v: &Version = &input;
        let was = v.header(&Default::default())?;
        let about = cx.get::<crate::code_rule_dsl::logic::program_image::About>()?;
        let mut known: Option<crate::code_rule_dsl::logic::front::Edges> = analyses::strata(v, &about).ok().map(|_| analyses::edges_by_name(v, &about));
        let mut names: BTreeSet<String> = BTreeSet::new();
        for r in v.rules() {
            for l in r.body() {
                if let L::Agg { atom, .. } = l.view()
                    && v.rel_name(atom.rel()).contains("rr_agg")
                {
                    names.insert(v.rel_name(atom.rel()).to_owned());
                }
            }
        }
        // The dependencies by name (`aggregate_demand::dependencies`): each head to the atoms its rules read, an
        // aggregate's included, and each native to what it reads.
        let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in v.rules() {
            let reads = r.body().filter_map(|l| match l.view() {
                L::Atom(a) | L::Agg { atom: a, .. } => Some(v.rel_name(a.rel()).to_owned()),
                _ => None,
            });
            deps.entry(v.rel_name(r.head().rel()).to_owned()).or_default().extend(reads);
        }
        for (name, native) in &was.natives {
            deps.entry(name.clone()).or_default().extend(native.reads().into_iter().map(str::to_owned));
        }
        let reach = Reach::default();
        // The rules that read each helper, the input's in order and then each demand rule as it is placed.
        #[derive(Clone, Copy)]
        enum Reader {
            In(RuleId),
            Demand(usize),
        }
        let mut readers: BTreeMap<String, Vec<Reader>> = BTreeMap::new();
        for r in v.rules() {
            let mut seen = BTreeSet::new();
            for l in r.body() {
                if let L::Atom(a) | L::Agg { atom: a, .. } = l.view()
                    && names.contains(v.rel_name(a.rel()))
                    && seen.insert(v.rel_name(a.rel()))
                {
                    readers.entry(v.rel_name(a.rel()).to_owned()).or_default().push(Reader::In(r.id()));
                }
            }
        }
        let mut p = Placement::default();
        let items_of = |p: &Placement, r: RuleId| -> Vec<Item> {
            let rule = v.rule(r).expect("a rule of the input");
            p.prefix.get(&r).map(|_| Item::Placed(r)).into_iter().chain(rule.body().map(|l| Item::Own(r, l.node()))).collect()
        };
        let mut decls = was.decls.clone();
        for name in names.iter() {
            let demand = format!("rr_demand_agg_{name}");
            if was.outputs.contains(name) || was.inputs.contains(name) || was.natives.contains_key(name.as_str()) || was.facts.contains_key(name.as_str()) || decls.contains_key(demand.as_str()) {
                continue;
            }
            let mut mask: Option<Vec<usize>> = None;
            let mut made: Vec<DemandRule> = Vec::new();
            let mut refused = false;
            'readers: for reader in readers.get(name).into_iter().flatten() {
                let (r, items) = match *reader {
                    Reader::In(r) => (r, items_of(&p, r)),
                    Reader::Demand(k) => {
                        // A demand rule reads no aggregate: one that reads the helper refuses it.
                        if p.demands[k].body.iter().any(|&i| AggregateDemand::rel(v, &p, i).is_some_and(|(rel, _, _)| rel == name)) {
                            refused = true;
                            break 'readers;
                        }
                        continue;
                    }
                };
                for &i in &items {
                    if AggregateDemand::rel(v, &p, i).is_none_or(|(rel, _, _)| rel != name) {
                        continue;
                    }
                    let Item::Own(_, n) = i else {
                        refused = true;
                        break 'readers;
                    };
                    let L::Agg { atom, over, .. } = lit_at(v, r, n).view() else {
                        refused = true;
                        break 'readers;
                    };
                    let positions: Vec<usize> = atom
                        .args()
                        .enumerate()
                        .filter_map(|(k, t)| match t {
                            TermRef::Var(s) if Some(s) != over => Some(k),
                            TermRef::Int(_) | TermRef::Str(_) => Some(k),
                            _ => None,
                        })
                        .collect();
                    if positions.is_empty() || mask.as_ref().is_some_and(|m| *m != positions) {
                        refused = true;
                        break 'readers;
                    }
                    mask = Some(positions.clone());
                    let mut keys = VarSet::new();
                    for (_, t) in atom.args().enumerate().filter(|(k, _)| positions.contains(k)) {
                        crate::code_rule_dsl::logic::plan::body::term_vars(t, &mut keys);
                    }
                    let Some(body) = guards(v, &p, r, &items, name, &keys, &deps, &reach) else {
                        refused = true;
                        break 'readers;
                    };
                    made.push(DemandRule { demand: demand.clone(), src: r, agg: n, positions, body });
                }
            }
            if refused || made.is_empty() {
                crate::code_rule_dsl::logic::profile::decision("aggregate demand", || format!("{name}: unchanged; no safe complete consumer key or incompatible consumers"));
                continue;
            }
            let positions = mask.expect("a consumer's keys");
            let guard_rels: BTreeSet<String> = made.iter().flat_map(|d| d.body.iter()).filter_map(|&i| AggregateDemand::rel(v, &p, i)).filter(|(_, _, agg)| !agg).map(|(rel, _, _)| rel.to_owned()).collect();
            let closes = known.as_ref().is_none_or(|edges| guard_rels.iter().any(|g| depends(edges, g, name))) || full_checks();
            // Placed: the demand declared, read first by the helper's rules, made by its demand rules.
            let decl: Vec<(String, String)> = positions.iter().map(|&k| { let (c, t) = &decls[name.as_str()][k]; (c.to_string(), t.to_string()) }).collect();
            let makers: Vec<RuleId> = v.rel(name).map(|rel| v.heads(rel).map(|r| r.id()).collect()).unwrap_or_default();
            let first = p.demands.len();
            decls.insert(demand.clone().into(), decl.iter().map(|(c, t)| (c.as_str().into(), t.as_str().into())).collect());
            p.decls.push((demand.clone(), decl));
            for &m in &makers {
                p.prefix.insert(m, (demand.clone(), positions.clone()));
            }
            p.demands.extend(made);
            if closes {
                let candidate = placed_version(&input, &was, &p)?;
                let about = crate::code_rule_dsl::logic::program_image::About::of(&candidate);
                match analyses::strata(&candidate, &about) {
                    Ok(_) => known = Some(analyses::edges_by_name(&candidate, &about)),
                    Err(_) => {
                        // Refused: taken out again, the program as it was.
                        p.demands.truncate(first);
                        for m in &makers {
                            p.prefix.remove(m);
                        }
                        p.decls.pop();
                        decls.remove(demand.as_str());
                        continue;
                    }
                }
            } else {
                let edges = known.as_mut().expect("the edges of a stratified program");
                edges.entry(name.clone()).or_default().insert((demand.clone(), false));
                edges.entry(demand.clone()).or_default().extend(guard_rels.iter().map(|g| (g.clone(), false)));
            }
            reach.placed(name, &demand, &guard_rels);
            deps.entry(name.clone()).or_default().insert(demand.clone());
            deps.entry(demand.clone()).or_default().extend(guard_rels);
            for k in first..p.demands.len() {
                let mut seen = BTreeSet::new();
                for &i in &p.demands[k].body {
                    if let Some((rel, _, _)) = AggregateDemand::rel(v, &p, i)
                        && names.contains(rel)
                        && seen.insert(rel.to_owned())
                    {
                        readers.entry(rel.to_owned()).or_default().push(Reader::Demand(k));
                    }
                }
            }
        }
        // Count zero is existence, provided its scalar result has no other consumer.
        let mut anti: BTreeMap<RuleId, (BTreeSet<u32>, BTreeSet<u32>)> = BTreeMap::new();
        for r in v.rules() {
            let mut head_vars = VarSet::new();
            for t in r.head().args() {
                crate::code_rule_dsl::logic::plan::body::term_vars(t, &mut head_vars);
            }
            let lits: Vec<_> = r.body().collect();
            for (i, l) in lits.iter().enumerate() {
                let L::Agg { var, op: crate::code_rule_dsl::logic::program_image::AggRef::Count, over: None, .. } = l.view() else { continue };
                let uses: Vec<_> = lits.iter().enumerate().filter(|(j, x)| *j != i && lit_vars(**x).contains(var)).collect();
                if head_vars.contains(var) || uses.len() != 1 {
                    continue;
                }
                let zero = matches!(uses[0].1.view(), L::Cmp { l: TermRef::Int(0), op: crate::code_rule_dsl::logic::program_image::CmpOp::Eq, r: TermRef::Var(x) } | L::Cmp { l: TermRef::Var(x), op: crate::code_rule_dsl::logic::program_image::CmpOp::Eq, r: TermRef::Int(0) } if x == var);
                if !zero {
                    continue;
                }
                let e = anti.entry(r.id()).or_default();
                e.0.insert(l.node());
                e.1.insert(uses[0].1.node());
            }
        }
        if p.prefix.is_empty() && p.demands.is_empty() && anti.is_empty() {
            return Ok(());
        }
        let mut head = was.clone();
        for (d, cols) in &p.decls {
            head.decls.insert(d.clone().into(), cols.iter().map(|(c, t)| (c.as_str().into(), t.as_str().into())).collect());
        }
        let none = (BTreeSet::new(), BTreeSet::new());
        let b = w.derive()?;
        b.set_header(&was, &head);
        for r in input.rules() {
            let (a, g) = anti.get(&r.id()).unwrap_or(&none);
            if !p.prefix.contains_key(&r.id()) && a.is_empty() {
                b.keep(&input, r);
            } else {
                write_placed(b, &input, &p, r, a, g)?;
            }
        }
        for d in &p.demands {
            write_demand(b, &input, &p, d)?;
        }
        Ok(())
    }
}

/// The program with what `p` places so far, for stratifying a candidate: an overlay of `input`, its helpers' rules
/// replaced by their placed forms and the demand rules added (the order is not the strata's concern).
fn placed_version(input: &std::sync::Arc<Version>, was: &crate::code_rule_dsl::logic::front::Program, p: &Placement) -> Result<std::sync::Arc<Version>, String> {
    let mut head = was.clone();
    for (d, cols) in &p.decls {
        head.decls.insert(d.clone().into(), cols.iter().map(|(c, t)| (c.as_str().into(), t.as_str().into())).collect());
    }
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(input);
    b.set_header(was, &head);
    let none = BTreeSet::new();
    for &r in p.prefix.keys() {
        b.remove(r);
        write_placed(&mut b, input, p, input.rule(r).expect("a rule of the input"), &none, &none)?;
    }
    for d in &p.demands {
        write_demand(&mut b, input, p, d)?;
    }
    b.finish_overlay()
}

/// `front::order_nodes` on the version (see there): under node ids per definition, two nodes compared by their
/// preorder keys (`node_order`), and a `min`/`max` over nodes taken over the keys through a helper
/// (`rr_ordered<k>`, written after the program's rules, as the tree pushes it). Fresh variables are numbered over
/// the program in rule order, as the tree's are.
pub struct OrderNodes;

/// What a literal of a rule becomes.
enum Ordered {
    Keep(u32),
    /// `x op y` over nodes: `node_order(x, kx), node_order(y, ky), kx op ky`.
    Cmp { x: Slot, y: Slot, op: crate::code_rule_dsl::logic::program_image::CmpOp, k: usize },
    /// `var = op o : { atom }` over a node `o`: `kv = op k : { helper(args, k) }, node_order(var, kv)`.
    Agg { lit: u32, var: Slot, o: Slot, op: crate::code_rule_dsl::logic::front::AggOp, k: usize, helper: usize, args: Vec<Slot> },
}

impl ProgramPass for OrderNodes {
    const NAME: &'static str = "order the nodes compared";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::front::AggOp;
        use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, CmpOp};
        if !crate::code_rule_dsl::logic::front::orders_nodes() {
            return Ok(());
        }
        let v = cx.v();
        let was = v.header(&Default::default())?;
        let typed = |a: AtomRef<'_>, nodes: &mut BTreeSet<Slot>| {
            let cols = was.decls.get(v.rel_name(a.rel())).map_or(&[][..], |c| c.as_slice());
            for (t, (_, ty)) in a.args().zip(cols) {
                if let TermRef::Var(s) = t
                    && ty == "node"
                {
                    nodes.insert(s);
                }
            }
        };
        let op_of = |op: AggRef<'_>| match op {
            AggRef::Min => Some(AggOp::Min),
            AggRef::Max => Some(AggOp::Max),
            _ => None,
        };
        let mut fresh = 0usize;
        let mut helpers = 0usize;
        let mut plans: BTreeMap<RuleId, Vec<Ordered>> = BTreeMap::new();
        for r in v.rules() {
            let mut nodes = BTreeSet::new();
            typed(r.head(), &mut nodes);
            for l in r.body() {
                match l.view() {
                    L::Atom(a) => typed(a, &mut nodes),
                    L::Agg { atom, .. } => typed(atom, &mut nodes),
                    L::Nearest { n, a, .. } => {
                        nodes.insert(n);
                        nodes.insert(a);
                    }
                    _ => {}
                }
            }
            // A min or max over nodes is a node.
            for l in r.body() {
                if let L::Agg { var, op, over: Some(o), atom } = l.view()
                    && op_of(op).is_some()
                {
                    let mut inner = BTreeSet::new();
                    typed(atom, &mut inner);
                    if inner.contains(&o) {
                        nodes.insert(var);
                    }
                }
            }
            if nodes.is_empty() {
                continue;
            }
            let mut plan = Vec::new();
            let mut changed = false;
            for l in r.body() {
                match l.view() {
                    L::Cmp { l: TermRef::Var(x), op, r: TermRef::Var(y) } if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) && nodes.contains(&x) && nodes.contains(&y) => {
                        plan.push(Ordered::Cmp { x, y, op, k: fresh });
                        fresh += 2;
                        changed = true;
                    }
                    L::Agg { var, op, over: Some(o), atom } if op_of(op).is_some() && {
                        let mut inner = BTreeSet::new();
                        typed(atom, &mut inner);
                        inner.contains(&o)
                    } =>
                    {
                        // The rows aggregated, each with its node's key: the atom's variables once each, but the node.
                        let mut args: Vec<Slot> = Vec::new();
                        for t in atom.args() {
                            if let TermRef::Var(s) = t
                                && s != o
                                && !args.contains(&s)
                            {
                                args.push(s);
                            }
                        }
                        plan.push(Ordered::Agg { lit: l.node(), var, o, op: op_of(op).expect("min or max"), k: fresh, helper: helpers, args });
                        fresh += 2;
                        helpers += 1;
                        changed = true;
                    }
                    _ => plan.push(Ordered::Keep(l.node())),
                }
            }
            if changed {
                plans.insert(r.id(), plan);
            }
        }
        if plans.is_empty() {
            return Ok(());
        }
        let node_order = crate::code_rule_dsl::logic::front::NODE_ORDER_REL;
        let ord = |k: usize| format!("rr_ord{k}");
        let mut head = was.clone();
        let input = std::sync::Arc::clone(w.input());
        // The helpers' declarations, in the order they are made.
        let mut made: Vec<(RuleId, u32, usize, Vec<Slot>, usize)> = Vec::new();
        for (id, plan) in &plans {
            for o in plan {
                if let Ordered::Agg { lit, k, helper, args, .. } = o {
                    made.push((*id, *lit, *helper, args.clone(), *k));
                }
            }
        }
        made.sort_by_key(|m| m.2);
        for (id, lit, helper, _, _) in &made {
            let L::Agg { atom, over: Some(o), .. } = lit_at(&input, *id, *lit).view() else { return Err("an ordered aggregate".into()) };
            let cols = was.decls.get(input.rel_name(atom.rel())).cloned().unwrap_or_default();
            let mut seen: Vec<Slot> = Vec::new();
            let mut decl: Vec<(String, String)> = Vec::new();
            for (i, t) in atom.args().enumerate() {
                if let TermRef::Var(s) = t
                    && s != o
                    && !seen.contains(&s)
                {
                    seen.push(s);
                    decl.push((format!("c{i}"), cols.get(i).map_or_else(|| "int".to_owned(), |(_, ty)| ty.to_string())));
                }
            }
            decl.push(("k".to_owned(), "int".to_owned()));
            head.decls.insert(format!("rr_ordered{helper}").into(), decl.iter().map(|(c, t)| (c.as_str().into(), t.as_str().into())).collect());
        }
        head.decls.entry(node_order.to_owned().into()).or_insert_with(|| [("n".into(), "node".into()), ("k".into(), "int".into())].into());
        if !head.inputs.iter().any(|i| i == node_order) {
            head.inputs.push(node_order.to_owned());
        }
        let names: Vec<String> = (0..helpers).map(|h| format!("rr_ordered{h}")).collect();
        let b = w.derive()?;
        b.set_header(&was, &head);
        use crate::code_rule_dsl::logic::program_image::build::t_of;
        for r in input.rules() {
            let Some(plan) = plans.get(&r.id()) else {
                b.keep(&input, r);
                continue;
            };
            let mut rb = b.rule().source(r.id()).head_from(&input, r, r.head());
            for o in plan {
                rb = match o {
                    Ordered::Keep(n) => rb.lit(&input, r, lit_at(&input, r.id(), *n)),
                    Ordered::Cmp { x, y, op, k } => {
                        let (kx, ky) = (ord(*k), ord(k + 1));
                        rb.atom(node_order, &[T::VarAtom(r.var(*x)), T::Var(&kx)]).atom(node_order, &[T::VarAtom(r.var(*y)), T::Var(&ky)]).cmp(T::Var(&kx), *op, T::Var(&ky))
                    }
                    Ordered::Agg { var, op, k, helper, args, .. } => {
                        let (kk, kv) = (ord(*k), ord(k + 1));
                        let mut agg_args: Vec<T<'_>> = args.iter().map(|s| T::VarAtom(r.var(*s))).collect();
                        agg_args.push(T::Var(&kk));
                        rb.agg(T::Var(&kv), op, Some(T::Var(&kk)), names[*helper].as_str(), &agg_args).atom(node_order, &[T::VarAtom(r.var(*var)), T::Var(&kv)])
                    }
                };
            }
            rb.finish()?;
        }
        // The helpers, after the program's rules: `rr_ordered<k>(args, k) :- atom, node_order(o, k)`.
        for (id, lit, helper, args, k) in &made {
            let r = input.rule(*id).expect("a rule of the input");
            let L::Agg { atom, over: Some(o), .. } = lit_at(&input, *id, *lit).view() else { return Err("an ordered aggregate".into()) };
            let kk = ord(*k);
            let mut head_args: Vec<T<'_>> = args.iter().map(|s| T::VarAtom(r.var(*s))).collect();
            head_args.push(T::Var(&kk));
            let atom_args: Vec<T<'_>> = atom.args().map(|t| t_of(&input, r, t)).collect();
            b.rule()
                .source(*id)
                .head(names[*helper].as_str(), &head_args)
                .atom_with(input.rel_name(atom.rel()), &atom_args, false, atom.gated())
                .atom(node_order, &[T::VarAtom(r.var(o)), T::Var(&kk)])
                .finish()?;
        }
        Ok(())
    }
}

/// `front::declare_axes` on the version: each tree axis a rule reads (an aggregate's atom included) that the program
/// does not declare, declared `(n: node, a: node)` and an input, in the order the rules first read them.
pub struct DeclareAxes;

impl ProgramPass for DeclareAxes {
    const NAME: &'static str = "declare the tree axes read";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let was = v.header(&Default::default())?;
        let mut head = was.clone();
        for r in v.rules() {
            for l in r.body() {
                let (L::Atom(a) | L::Agg { atom: a, .. }) = l.view() else { continue };
                let rel = v.rel_name(a.rel());
                if crate::code_rule_dsl::logic::front::is_axis(rel) && !head.decls.contains_key(rel) {
                    head.decls.insert(rel.to_owned().into(), [("n".into(), "node".into()), ("a".into(), "node".into())].into());
                    head.inputs.push(rel.to_owned());
                }
            }
        }
        if head.decls.len() != was.decls.len() {
            w.on()?.set_header(&was, &head);
        }
        Ok(())
    }
}
