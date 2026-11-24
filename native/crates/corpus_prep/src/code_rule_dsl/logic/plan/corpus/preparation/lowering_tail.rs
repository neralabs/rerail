//! The lowering's whole-program half on the version (decision 0281's path, step 7b; 0246 §1, "whole program, at
//! link time"): the findings' rules, core's declarations, projections, kind sets, boolean relations, the rewrites and checks
//! of `front::finish_checks_on`, aggregate demand, removing repeated rules and literals, and widening cyclic contexts. Every
//! text runs them (`rr::link_library_versions`, `rr::link_seeded_version`); their tree forms went with the tree's
//! lowering (decision 0276 step 5).

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::logic::plan::body::{Body, VarSet, lit_vars};
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{Lit as L, RuleId, Slot, TermRef, Version};
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};
use std::collections::{BTreeMap, BTreeSet};

/// A context relation (`Lowerer::context`'s `rr_ctx<k>`) narrows where a helper is computed to the values its rule
/// binds. Built from a recursive rule's literals, it reads the rule's own component, and a negated or aggregated
/// helper it narrows then sits in that component: `has_of(n) :- demand_has_of(n), …, count : { … } = 0` with
/// `demand_has_of` asked by `has_of` is not stratifiable, though the helper's rows at the rule's values are not in
/// doubt. One round: each context in a component that a negation or an aggregate closes a cycle in drops the literals
/// of that component, and of the comparisons and negations only those whose variables the rest still binds (an
/// aggregate is left out); a node column the rest leaves unbound ranges over the nodes (`node(_, v, _, _, _)`). The
/// context then holds more rows than before, never fewer, and every rule that reads it joins it with its own literals,
/// so no row it gives changes. `changed` says whether the round widened one; the caller runs rounds until none does
/// (16 at most).
#[derive(Default)]
pub struct WidenCyclicContexts {
    pub changed: bool,
}

/// A widened rule: its kept literals by node, in order, and the head variables then ranged over `node`.
struct Widened {
    lits: Vector<u32>,
    nodes: Vec<Slot>,
}

impl ProgramPass for WidenCyclicContexts {
    const NAME: &'static str = "widen the cyclic contexts";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let about = cx.get::<crate::code_rule_dsl::logic::program_image::About>()?;
        let errors = crate::code_rule_dsl::logic::program_image::analyses::unstratified(v, &about);
        let sccs: Vec<BTreeSet<String>> = errors.iter().filter_map(|e| e.unstratified().map(|(scc, _)| scc.clone())).collect();
        if sccs.is_empty() {
            return Ok(());
        }
        let node_declared = v.declares("node");
        let mut widened: BTreeMap<RuleId, Widened> = BTreeMap::new();
        for scc in &sccs {
            for rule in v.rules() {
                let head = v.rel_name(rule.head().rel());
                if crate::code_rule_dsl::logic::rr::lower::helper_stem(head) != Some("ctx") || !scc.contains(head) {
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
                let mut lits: Vector<u32> = kept.iter().filter(|l| binds(&l.view())).map(|l| l.node()).collect();
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
                let cols = v.columns_of(head);
                let mut types: BTreeMap<Slot, &str> = BTreeMap::new();
                let mut head_vars: BTreeMap<String, Slot> = BTreeMap::new();
                for (k, t) in rule.head().args().enumerate() {
                    if let Some(s) = t.var()
                        && let Some((_, ty)) = cols.as_ref().and_then(|c| c.get(k)) {
                            types.insert(s, ty.as_str());
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
        let mut once: BTreeMap<RuleId, Vector<u32>> = BTreeMap::new();
        for rule in v.rules() {
            let shown: Vec<(u32, String)> = rule.body().map(|l| (l.node(), v.show_lit(rule, l))).collect();
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let kept: Vector<u32> = shown.iter().filter(|(_, s)| seen.insert(s.as_str())).map(|(n, _)| *n).collect();
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
        let v: &Version = cx.v();
        let mut once = Vec::new();
        for rel in ProgramFields::fact_names(v) {
            let rows = ProgramFields::fact_rows(v, &rel);
            let mut kept: Vec<crate::code_rule_dsl::logic::front::List<crate::code_rule_dsl::logic::front::Value>> = Vec::with_capacity(rows.len());
            for row in &rows {
                if !kept.contains(row) {
                    kept.push(row.clone());
                }
            }
            if kept.len() != rows.len() {
                once.push((rel, kept));
            }
        }
        if !once.is_empty() {
            let b = w.on()?;
            for (rel, rows) in &once {
                b.fields().set_facts(rel, rows);
            }
        }
        Ok(())
    }
}

/// A rule that reads no relation (an aggregate's helper of comparisons only, `{ e = "a", q = @json_str(e) }`) holds
/// once, as in Soufflé: it reads `rr_unit(0)` first, one row, since the planner starts every body from an atom, and
/// `rr_unit` is declared with its one fact. A matcher atom or a walk reads the tree itself; an aggregate reads its
/// atom. A body with an aggregate (`ok(v) :- m = join …, v = @f(m).`) needs none: the planner runs a body of filters
/// and aggregates over the relational unit, and the SQL executor maintains it as a scalar rule over its aggregates'
/// inputs (a unit atom would hide them from its retained support).
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
        let input = std::sync::Arc::clone(w.input());
        let b = w.derive()?;
        b.fields().declare(UNIT, &[("x", "int")]);
        b.fields().set_facts(UNIT, &[[crate::code_rule_dsl::logic::front::Value::Int(0)].into()]);
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

/// Aggregate demand (`rr::aggregate_demand`'s analyses): each private aggregate helper (`rr_agg…`) restricted
/// to the keys its consumers ask (`rr_demand_agg_<helper>`, read first by its rules and made from each consumer's
/// guards), placed one helper at a time and refused where it makes the program unstratifiable; then a `count` of
/// zero read once made an anti-join. The tree's order is kept: a helper's rules gain their demand where they stand,
/// and the demand rules follow the program's.
///
/// The program is taken to be declared: `lowering_tail::Declared` checks a text's before this runs.
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
    positions: Vector<usize>,
    body: Vec<Item>,
}

/// What `optimize` has placed so far.
#[derive(Default)]
struct Placement {
    /// A helper's rule: the demand it reads first, over its head's columns `positions`.
    prefix: BTreeMap<RuleId, (String, Vector<usize>)>,
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

/// Safe positive guards, then comparisons whose inputs those guards bind (a context is a superset of the consumer,
/// never an assumption about a not-yet-computed aggregate or recursive result), over rule `r`'s items: its safe positive guards joined to the keys, then the
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
        let args: Vec<T<'_>> = r.head().args().enumerate().filter(|(k, _)| positions.contains(k)).map(|(_, t)| t_of(r, t)).collect();
        rb = rb.atom(demand.as_str(), &args);
    }
    for l in r.body() {
        if gone.contains(&l.node()) {
            continue;
        }
        rb = match (anti.contains(&l.node()), l.view()) {
            (true, L::Agg { atom, .. }) => {
                let args: Vec<T<'_>> = atom.args().map(|t| t_of(r, t)).collect();
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
    let args: Vec<T<'_>> = atom.args().enumerate().filter(|(k, _)| d.positions.contains(k)).map(|(_, t)| t_of(src, t)).collect();
    let mut rb = b.rule().source(d.src).head(d.demand.as_str(), &args);
    for &i in &d.body {
        rb = match i {
            Item::Own(r, n) => rb.lit(input, input.rule(r).expect("a rule"), lit_at(input, r, n)),
            Item::Placed(r) => {
                let rule = input.rule(r).expect("a rule");
                let (demand, positions) = &p.prefix[&r];
                let args: Vec<T<'_>> = rule.head().args().enumerate().filter(|(k, _)| positions.contains(k)).map(|(_, t)| t_of(rule, t)).collect();
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
        let natives = v.native_map()?;
        let about = cx.get::<crate::code_rule_dsl::logic::program_image::About>()?;
        let mut known: Option<crate::code_rule_dsl::logic::front::Edges> = analyses::strata(v, &about).ok().map(|_| analyses::edges_by_name(v, &about));
        let mut names: BTreeSet<String> = BTreeSet::new();
        for r in v.rules() {
            for l in r.body() {
                if let L::Agg { atom, .. } = l.view()
                    && crate::code_rule_dsl::logic::rr::lower::holds_helper(v.rel_name(atom.rel()), &["agg"])
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
        for (name, native) in &natives {
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
        for name in names.iter() {
            let demand = format!("rr_demand_agg_{name}");
            if v.has_output(name) || v.has_input(name) || natives.contains_key(name.as_str()) || ProgramFields::states_facts(v, name) || v.declares(&demand) || p.decls.iter().any(|(d, _)| *d == demand) {
                continue;
            }
            let mut mask: Option<rerail_blocks::vector::Vector<usize>> = None;
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
                    let positions: rerail_blocks::vector::Vector<usize> = atom
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
            let cols = v.columns_of(name).ok_or_else(|| format!("{name}: an aggregate's helper the program does not declare"))?;
            let decl: Vec<(String, String)> = positions.iter().map(|&k| cols[k].clone()).collect();
            let makers: Vec<RuleId> = v.rel(name).map(|rel| v.heads(rel).map(|r| r.id()).collect()).unwrap_or_default();
            let first = p.demands.len();
            p.decls.push((demand.clone(), decl));
            for &m in &makers {
                p.prefix.insert(m, (demand.clone(), positions.clone()));
            }
            p.demands.extend(made);
            if closes {
                let candidate = placed_version(&input, &p)?;
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
        let none = (BTreeSet::new(), BTreeSet::new());
        let b = w.derive()?;
        declare_placed(b, &p);
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
fn placed_version(input: &std::sync::Arc<Version>, p: &Placement) -> Result<std::sync::Arc<Version>, String> {
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(input);
    declare_placed(&mut b, p);
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

/// The demand relations `p` places, declared.
fn declare_placed(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, p: &Placement) {
    for (d, cols) in &p.decls {
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        b.fields().declare(d, &pairs);
    }
}

/// Rules that order nodes, under node ids per definition (decision 0094 §2). An id follows preorder only inside its
/// granule, so a comparison of two nodes (`b < n`: a binding before a use) and a `min`/`max` over nodes read the
/// nodes' preorder keys instead (`node_order`): the order preorder ids gave, so the rule means what it did.
/// - `x < y`, both nodes: `kx = @rr_node_order(x), ky = @rr_node_order(y), kx < ky` (`<=`, `>`, `>=` likewise):
///   each node's key read off the tape where its batch holds it, as `node_order`'s `k` is, not two joins with
///   `node_order` (t57 §8, E2);
/// - `v = max x : { r(…x…) }`, `x` a node: `kv = max k : { r_ord(…, _, k) }, r_ord(…, v, kv)`, where
///   `r_ord(…, x, k) :- r(…x…), node_order(x, k)`: a helper `rr_ordered<k>`, written after the program's rules.
///   `v` is the aggregated row's own node, read back from the helper at the key, not `node_order(v, kv)`: the key
///   names one node (decision 0121's keys hold the document), but the split cannot see a document in a number, and
///   took that rule for a join of two documents (decision 0652). The helper's row holds the group and its node
///   together, as the aggregated atom did. Its columns bound only inside the aggregate are `_` there.
///
/// A variable is a node when a column it fills is declared `node`: in the head or any atom of the body. Fresh
/// variables are numbered over the program in rule order.
pub struct OrderNodes;

/// The node read of a node's preorder key (`vtab::NODE_READS`).
const NODE_ORDER_READ: &str = "rr_node_order";

/// What a literal of a rule becomes.
enum Ordered {
    Keep(u32),
    /// `x op y` over nodes: `kx = @rr_node_order(x), ky = @rr_node_order(y), kx op ky`.
    Cmp { x: Slot, y: Slot, op: crate::code_rule_dsl::logic::program_image::CmpOp, k: usize },
    /// `var = op o : { atom }` over a node `o`: `kv = op k : { helper(args, _, k) }, helper(args', var, kv)`, `args'`
    /// the arguments bound outside the aggregate (`outer`), the others `_`.
    Agg { lit: u32, var: Slot, o: Slot, op: crate::code_rule_dsl::logic::front::AggOp, k: usize, helper: usize, args: Vec<Slot>, outer: Vec<bool> },
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
        let typed = |a: AtomRef<'_>, nodes: &mut BTreeSet<Slot>| {
            let cols = v.columns_of(v.rel_name(a.rel())).unwrap_or_default();
            for (t, (_, ty)) in a.args().zip(&cols) {
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
                        // The arguments the rule binds outside the aggregate: its head's and its other literals'.
                        let mut outside = crate::code_rule_dsl::logic::plan::body::VarSet::new();
                        for t in r.head().args() {
                            crate::code_rule_dsl::logic::plan::body::term_vars(t, &mut outside);
                        }
                        for other in r.body().filter(|x| x.node() != l.node()) {
                            outside.extend(&lit_vars(other));
                        }
                        let outer = args.iter().map(|&s| outside.contains(s)).collect();
                        plan.push(Ordered::Agg { lit: l.node(), var, o, op: op_of(op).expect("min or max"), k: fresh, helper: helpers, args, outer });
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
        let input = std::sync::Arc::clone(w.input());
        let mut declared: Vec<(String, Vec<(String, String)>)> = Vec::new();
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
            let cols = input.columns_of(input.rel_name(atom.rel())).unwrap_or_default();
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
            decl.push(("n".to_owned(), "node".to_owned()));
            decl.push(("k".to_owned(), "int".to_owned()));
            declared.push((format!("rr_ordered{helper}"), decl));
        }
        let names: Vec<String> = (0..helpers).map(|h| format!("rr_ordered{h}")).collect();
        let b = w.derive()?;
        for (rel, decl) in &declared {
            let pairs: Vec<(&str, &str)> = decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            b.fields().declare(rel, &pairs);
        }
        // Declared as T's front end declares it (`front::tests::t_programs_match_t_front_end`): an ordered aggregate's
        // helper reads it; a comparison reads the key in place (`@rr_node_order`), and an input no rule reads is
        // never made.
        b.fields().declare_absent(node_order, &[("n", "node"), ("k", "int")]);
        b.fields().add_input(node_order);
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
                    // Each node's key read where the node lies (`@rr_node_order`), then compared: no join. A key read
                    // into a variable is a node read the split moves to the per-file part with its node (`push`), as
                    // it moved `node_order`'s row: a position stays there (decision 0649).
                    Ordered::Cmp { x, y, op, k } => {
                        let (kx, ky) = (ord(*k), ord(k + 1));
                        rb.cmp(T::Var(&kx), CmpOp::Eq, T::Call(NODE_ORDER_READ, vec![T::VarAtom(r.var(*x))]))
                            .cmp(T::Var(&ky), CmpOp::Eq, T::Call(NODE_ORDER_READ, vec![T::VarAtom(r.var(*y))]))
                            .cmp(T::Var(&kx), *op, T::Var(&ky))
                    }
                    Ordered::Agg { var, op, k, helper, args, outer, .. } => {
                        let (kk, kv) = (ord(*k), ord(k + 1));
                        let mut agg_args: Vec<T<'_>> = args.iter().map(|s| T::VarAtom(r.var(*s))).collect();
                        agg_args.push(T::Wild);
                        agg_args.push(T::Var(&kk));
                        let mut at_args: Vec<T<'_>> = args.iter().zip(outer).map(|(s, &o)| if o { T::VarAtom(r.var(*s)) } else { T::Wild }).collect();
                        at_args.push(T::VarAtom(r.var(*var)));
                        at_args.push(T::Var(&kv));
                        rb.agg(T::Var(&kv), op, Some(T::Var(&kk)), names[*helper].as_str(), &agg_args).atom(names[*helper].as_str(), &at_args)
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
            head_args.push(T::VarAtom(r.var(o)));
            head_args.push(T::Var(&kk));
            let atom_args: Vec<T<'_>> = atom.args().map(|t| t_of(r, t)).collect();
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

/// A `nearest` read from its near end (t57 §8, E1). In a recursive rule whose recursive atom binds a `nearest`'s far
/// end (its ancestor) and not its near end, the plan starts from the recursion's new rows (the delta) and reads the
/// `nearest` backwards: every node whose nearest ancestor of the set is the bound one, its whole body. `rf_qn(f, q) :-
/// rf_def(f, x), nearest(f, s, rf_dc), kind(s, …), rf_qn(s, p), …` listed 4,528 nodes under 96 scopes to keep their
/// definitions. The part that reads the near end, the `nearest` with the positive atoms that bind its near end and
/// read nothing the recursion binds, is made once as a helper before the recursion, where its plan binds the near end
/// first (`rr_near<k>(s, f, x) :- rf_def(f, x), rr_nearest_rf_dc(f, s)`), and the recursion reads the helper by the
/// far end. Its columns are the part's variables the rest of the rule reads; it holds a row a row of the atoms that
/// bind the near end (a node has one nearest ancestor of a set). The conjunction is the same, so no row changes.
pub struct NearEnds;

impl ProgramPass for NearEnds {
    const NAME: &'static str = "nearest from its near end";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::front::{NEAREST, is_transduced};
        use crate::code_rule_dsl::logic::plan::body::term_vars;
        let v = cx.v();
        // Which relations each relation's rules read: a recursive atom's relation reaches its rule's head.
        let mut reads: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for r in v.rules() {
            let e = reads.entry(v.rel_name(r.head().rel())).or_default();
            for l in r.body() {
                match l.view() {
                    L::Atom(a) => {
                        e.insert(v.rel_name(a.rel()));
                    }
                    L::Agg { atom, .. } => {
                        e.insert(v.rel_name(atom.rel()));
                    }
                    _ => {}
                }
            }
        }
        let reaches = |from: &str, to: &str| -> bool {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack = vec![from];
            while let Some(x) = stack.pop() {
                if x == to {
                    return true;
                }
                if seen.insert(x) {
                    stack.extend(reads.get(x).into_iter().flatten().copied());
                }
            }
            false
        };
        // By rule: the literals the helper takes (by node), its columns, and its number.
        let mut parts: BTreeMap<RuleId, Vec<(Vec<u32>, Vec<Slot>, usize)>> = BTreeMap::new();
        let mut made = 0usize;
        // A part by what it reads, its variables numbered as they first appear, and its columns: rules with the same
        // part (`rf_qn`'s rule for a class's definitions and for a function's) read one helper.
        let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
        // The helpers made, each the rule and part that writes it.
        let mut writers: Vec<(RuleId, usize)> = Vec::new();
        for r in v.rules() {
            let head = v.rel_name(r.head().rel());
            let lits: Vec<_> = r.body().collect();
            fn positive_atom<'a>(l: &crate::code_rule_dsl::logic::program_image::LitRef<'a>) -> Option<crate::code_rule_dsl::logic::program_image::AtomRef<'a>> {
                match l.view() {
                    L::Atom(a) if !a.neg() => Some(a),
                    _ => None,
                }
            }
            // A `nearest` literal's near end and far end, written as the literal or as its relation's atom.
            let ends = |l: &crate::code_rule_dsl::logic::program_image::LitRef<'_>| -> Option<(Slot, Slot)> {
                match l.view() {
                    L::Nearest { n, a, .. } => Some((n, a)),
                    L::Atom(a) if !a.neg() && v.rel_name(a.rel()).starts_with(NEAREST) => match a.args().collect::<Vec<_>>()[..] {
                        [TermRef::Var(n), TermRef::Var(far)] => Some((n, far)),
                        _ => None,
                    },
                    _ => None,
                }
            };
            if !lits.iter().any(|l| ends(l).is_some()) {
                continue;
            }
            let recursive: Vec<u32> = lits
                .iter()
                .filter(|l| positive_atom(l).is_some_and(|a| !is_transduced(v.rel_name(a.rel())) && reaches(v.rel_name(a.rel()), head)))
                .map(|l| l.node())
                .collect();
            if recursive.is_empty() {
                continue;
            }
            let mut rec_vars = VarSet::new();
            for l in lits.iter().filter(|l| recursive.contains(&l.node())) {
                rec_vars.extend(&lit_vars(*l));
            }
            let mut taken: BTreeSet<u32> = BTreeSet::new();
            let mut rule_parts = Vec::new();
            for l in &lits {
                let Some((n, far)) = ends(l) else { continue };
                if !rec_vars.contains(far) || rec_vars.contains(n) || taken.contains(&l.node()) {
                    continue;
                }
                // The atoms that bind the near end and read nothing the recursion binds.
                let binders: Vec<u32> = lits
                    .iter()
                    .filter(|o| o.node() != l.node() && !taken.contains(&o.node()) && !recursive.contains(&o.node()))
                    .filter(|o| positive_atom(o).is_some_and(|b| !is_transduced(v.rel_name(b.rel()))))
                    .filter(|o| {
                        let vs = lit_vars(**o);
                        vs.contains(n) && vs.is_disjoint(&rec_vars)
                    })
                    .map(|o| o.node())
                    .collect();
                if binders.is_empty() {
                    continue;
                }
                let mut part: Vec<u32> = binders;
                part.push(l.node());
                // Its columns: its variables the head or the rule's other literals read, in the order the part names them.
                let mut outside = VarSet::new();
                for t in r.head().args() {
                    term_vars(t, &mut outside);
                }
                for o in lits.iter().filter(|o| !part.contains(&o.node())) {
                    outside.extend(&lit_vars(*o));
                }
                let mut cols: Vec<Slot> = Vec::new();
                for o in lits.iter().filter(|o| part.contains(&o.node())) {
                    for s in lit_vars(*o).iter() {
                        if outside.contains(s) && !cols.contains(&s) {
                            cols.push(s);
                        }
                    }
                }
                taken.extend(part.iter().copied());
                let key = {
                    let mut seen: Vec<Slot> = Vec::new();
                    let mut num = |s: Slot| match seen.iter().position(|&x| x == s) {
                        Some(i) => i,
                        None => {
                            seen.push(s);
                            seen.len() - 1
                        }
                    };
                    let mut key = String::new();
                    for o in lits.iter().filter(|o| part.contains(&o.node())) {
                        match o.view() {
                            L::Nearest { n, a, kindset } => key += &format!("nearest({}, {}, {kindset:?}) ", num(n), num(a)),
                            L::Atom(a) => {
                                key += v.rel_name(a.rel());
                                for t in a.args() {
                                    key += &match t {
                                        TermRef::Var(s) => format!(" v{}", num(s)),
                                        t => format!(" {}", v.show_term(r, t)),
                                    };
                                }
                                key += "; ";
                            }
                            _ => {}
                        }
                    }
                    let cols: Vec<usize> = cols.iter().map(|&s| num(s)).collect();
                    format!("{key}-> {cols:?}")
                };
                let k = *by_key.entry(key).or_insert_with(|| {
                    writers.push((r.id(), rule_parts.len()));
                    made += 1;
                    made - 1
                });
                rule_parts.push((part, cols, k));
            }
            if !rule_parts.is_empty() {
                parts.insert(r.id(), rule_parts);
            }
        }
        if parts.is_empty() {
            return Ok(());
        }
        let name = |k: usize| format!("rr_near{k}");
        let input = std::sync::Arc::clone(w.input());
        // Each helper's columns, typed by the first column of a part's atom each variable fills.
        let mut declared: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for &(id, i) in &writers {
            let r = input.rule(id).expect("a rule of the input");
            {
                let (part, cols, k) = &parts[&id][i];
                let decl = cols
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| {
                        let ty = part.iter().find_map(|&n| {
                            let a = match lit_at(&input, r.id(), n).view() {
                                L::Atom(a) => a,
                                L::Nearest { n, a, .. } => return (s == n || s == a).then(|| "node".to_owned()),
                                _ => return None,
                            };
                            let at = a.args().position(|t| matches!(t, TermRef::Var(x) if x == s))?;
                            input.columns_of(input.rel_name(a.rel())).and_then(|c| c.get(at).map(|(_, ty)| ty.to_string()))
                        });
                        (format!("c{i}"), ty.unwrap_or_else(|| "int".to_owned()))
                    })
                    .collect();
                declared.push((name(*k), decl));
            }
        }
        let names: Vec<String> = (0..made).map(name).collect();
        let b = w.derive()?;
        for (rel, decl) in &declared {
            let pairs: Vec<(&str, &str)> = decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            b.fields().declare(rel, &pairs);
        }
        for r in input.rules() {
            let Some(rule_parts) = parts.get(&r.id()) else {
                b.keep(&input, r);
                continue;
            };
            let mut rb = b.rule().source(r.id()).head_from(&input, r, r.head());
            for l in r.body() {
                match rule_parts.iter().find(|(part, _, _)| part.contains(&l.node())) {
                    // The helper's atom where the part's `nearest` was.
                    Some((part, cols, k)) if part.last() == Some(&l.node()) => {
                        let args: Vec<T<'_>> = cols.iter().map(|&s| T::VarAtom(r.var(s))).collect();
                        rb = rb.atom(names[*k].as_str(), &args);
                    }
                    Some(_) => {}
                    None => rb = rb.lit(&input, r, l),
                }
            }
            rb.finish()?;
        }
        // The helpers, after the program's rules: `rr_near<k>(cols) :- binders, nearest`.
        for &(id, i) in &writers {
            let r = input.rule(id).expect("a rule of the input");
            {
                let (part, cols, k) = &parts[&id][i];
                let args: Vec<T<'_>> = cols.iter().map(|&s| T::VarAtom(r.var(s))).collect();
                let mut rb = b.rule().source(id).head(names[*k].as_str(), &args);
                for &n in part {
                    rb = rb.lit(&input, r, lit_at(&input, r.id(), n));
                }
                rb.finish()?;
            }
        }
        Ok(())
    }
}

/// The tree axes' relations a rule reads (an aggregate's atom included) and the program does not declare, declared
/// `(n: node, a: node)` and inputs, in the order the rules first read them: the transducer makes them.
pub struct DeclareAxes;

impl ProgramPass for DeclareAxes {
    const NAME: &'static str = "declare the tree axes read";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let mut axes: Vec<&str> = Vec::new();
        for r in v.rules() {
            for l in r.body() {
                let (L::Atom(a) | L::Agg { atom: a, .. }) = l.view() else { continue };
                let rel = v.rel_name(a.rel());
                if crate::code_rule_dsl::logic::front::is_axis(rel) && !v.declares(rel) && !axes.contains(&rel) {
                    axes.push(rel);
                }
            }
        }
        if !axes.is_empty() {
            let b = w.on()?;
            for rel in axes {
                b.fields().declare(rel, &[("n", "node"), ("a", "node")]);
                b.fields().add_input(rel);
            }
        }
        Ok(())
    }
}

/// A nullary relation is a Boolean set: empty, or the single empty tuple. SQL tables need a column, so the tuple is
/// the integer unit: the relation gains a column `rr_unit`, its facts a `1`, and each atom of it (a head, a read, an
/// aggregate's) a `1`; joins and negation then keep ordinary relational set semantics.
pub struct BooleanRelations;

/// Appends the unit to an atom of a nullary relation.
struct Unit1<'a> {
    v: &'a Version,
    names: &'a BTreeSet<String>,
    touched: &'a BTreeSet<RuleId>,
}

impl crate::code_rule_dsl::logic::program_image::build::Rewrite for Unit1<'_> {
    fn begin(&mut self, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>) -> bool {
        self.touched.contains(&r.id())
    }
    fn prefix(&mut self, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>, out: &mut Vec<T<'static>>) {
        if self.names.contains(self.v.rel_name(a.rel())) && a.args().next().is_none() {
            out.push(T::Int(1));
        }
    }
}

impl ProgramPass for BooleanRelations {
    const NAME: &'static str = "nullary relations as sets of the unit";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v: &Version = cx.v();
        let names: BTreeSet<String> = v.decls().into_iter().filter(|d| d.cols().next().is_none()).map(|d| v.text(d.name()).to_owned()).collect();
        if names.is_empty() {
            return Ok(());
        }
        let of = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| names.contains(v.rel_name(a.rel())) && a.args().next().is_none();
        let touched: BTreeSet<RuleId> = v
            .rules()
            .filter(|r| of(r.head()) || r.body().any(|l| matches!(l.view(), L::Atom(a) | L::Agg { atom: a, .. } if of(a))))
            .map(|r| r.id())
            .collect();
        let input = std::sync::Arc::clone(w.input());
        let mut nullary = v.nullary_names()?;
        nullary.extend(names.iter().cloned());
        let had: Vec<(String, Vec<crate::code_rule_dsl::logic::front::List<crate::code_rule_dsl::logic::front::Value>>)> =
            names.iter().filter(|n| ProgramFields::states_facts(v, n)).map(|n| (n.clone(), ProgramFields::fact_rows(v, n))).collect();
        let b = w.derive()?;
        b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Nullary(&nullary));
        for name in &names {
            b.fields().declare(name, &[("rr_unit", "int")]);
        }
        for (name, rows) in &had {
            let rows: Vec<_> = rows.iter().map(|row| row.iter().cloned().chain([crate::code_rule_dsl::logic::front::Value::Int(1)]).collect::<Vec<_>>().into()).collect();
            b.fields().set_facts(name, &rows);
        }
        b.rewrite_all(&input, &mut Unit1 { v: &input, names: &names, touched: &touched })?;
        Ok(())
    }
}

/// The documents and texts a program's `.scope` settings select (`lower::scope_text_globs`, `lower::scope_globs`),
/// set in its header as `whole()` sets them: a printed program read again selects the same.
pub struct ScopeGlobs;

impl ProgramPass for ScopeGlobs {
    const NAME: &'static str = "the scope's globs";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::{reads_tool_calls, scope_globs, scope_text_globs};
        let v = cx.v();
        let settings = v.settings()?;
        let (was_globs, was_scopes) = (v.text_globs()?, v.doc_scopes()?);
        let globs = scope_text_globs(&settings);
        // A program of tool calls selects no document: no entry reads a path (main's #866, `.scope tool_calls`).
        let scopes = if reads_tool_calls(&settings) { Some(Vec::new()) } else { was_scopes.clone().or_else(|| scope_globs(&settings).map(|g| vec![g])) };
        if globs != was_globs {
            w.on()?.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::TextGlobs(&globs));
        }
        if scopes != was_scopes {
            w.on()?.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::DocScopes(scopes.as_deref()));
        }
        Ok(())
    }
}

/// Kind sets: the relations of facts given to `nearest`, `inside` and `has`, over the sets the clauses named (the
/// lowerer's `sets`). Each set whose kinds are known now (its facts, and the kinds of `s` for each rule
/// `set(k) :- s(k)`: `kinds_of`) is a kind set of the header; one read nowhere else is only a set, taken out (to a
/// fixed point). A set of kinds that rules derive is no kind set: the transducer cannot read it from a node's kind,
/// so its axes are rules over `child`, written after the program's (`derived_axes`), in which each node's membership
/// (`rr_in_<s>(x) :- node(_, x, k, _, _), s(k)`) is read once. A helper reads its set negated, so the set is complete
/// before it (a lower stratum; a set that depends on its own axis is refused by the strata):
/// - `rr_near_<s>(n, a)`, a is n's nearest strict ancestor in s (`nearest`, and `inside(n, _, s)`): down from each
///   member to the next, a row per node below one;
/// - `rr_anc_<s>(n, a)`, every strict ancestor a of n in s (`inside`): the chain of nearest ones;
/// - `rr_desc_<s>(n, a)`, n in s and a any strict ancestor of it (`has`): up from each member;
/// - `rr_above_<s>(a)`, a has a member strictly below it (`has(a, _, s)`).
///
/// (A set of nodes is read as climbs where it is given, `node_axis`.)
pub struct KindSets {
    pub sets: BTreeSet<String>,
    /// The linked library of a module lowered in its scope (`rr::Library::version`): a set it reads has the library's
    /// kinds first (`kinds_of`).
    pub library: Option<std::sync::Arc<Version>>,
}

/// The kinds of a unary relation of kinds: its facts, and the kinds of `s` for each rule `set(k) :- s(k)`, rules gone
/// by head left out. A relation of the linked `library` of a module lowered in its scope (declared here, its facts and
/// rules not held) has its kinds there first, as the whole program read alone has them: its kind set, else its facts
/// and rules (python/names.rerail's `scope`; eslint-a-control.rerail's `breakable_kind(k) :- loop_kind(k)`; a program's own
/// `fn_stop(k) :- function_kind(k)` over one: PR #508's afeb076e5 and 4146acbd9, t47).
fn kinds_of(v: &Version, head: &(impl ProgramFields + ?Sized), library: Option<(&Version, &BTreeMap<String, Vec<String>>)>, gone: &BTreeSet<String>, set: &str, seen: &mut BTreeSet<String>) -> Option<Vec<String>> {
    use crate::code_rule_dsl::logic::front::Value;
    if !seen.insert(set.to_owned()) || head.input_names().iter().any(|i| i == set) || head.columns_of(set).is_some_and(|c| c.len() != 1) {
        return None;
    }
    let mut kinds: Vec<String> = Vec::new();
    // `set`'s facts and its rules `set(k) :- s(k)` in `of`, each `s` read through `kinds_of` again.
    let from = |of: &Version, facts: Vec<crate::code_rule_dsl::logic::front::List<Value>>, gone: &BTreeSet<String>, kinds: &mut Vec<String>, seen: &mut BTreeSet<String>| -> Option<()> {
        for row in facts {
            let [Value::Str(k)] = row.as_slice() else { return None };
            kinds.push(k.to_string());
        }
        for r in of.rel(set).into_iter().flat_map(|rel| of.heads(rel)).filter(|_| !gone.contains(set)) {
            let mut args = r.head().args();
            let (Some(TermRef::Var(k)), None) = (args.next(), args.next()) else { return None };
            let mut body = r.body();
            let (Some(l), None) = (body.next(), body.next()) else { return None };
            let L::Atom(a) = l.view() else { return None };
            let mut aargs = a.args();
            if a.neg() || !matches!((aargs.next(), aargs.next()), (Some(TermRef::Var(x)), None) if x == k) {
                return None;
            }
            kinds.extend(kinds_of(v, head, library, gone, of.rel_name(a.rel()), seen)?);
        }
        Some(())
    };
    if let Some((lib, sets)) = library.filter(|(lib, _)| lib.declares(set)) {
        match sets.get(set) {
            Some(set_kinds) => kinds.extend(set_kinds.iter().cloned()),
            None => from(lib, ProgramFields::fact_rows(lib, set), &BTreeSet::new(), &mut kinds, seen)?,
        }
    }
    from(v, head.fact_rows(set), gone, &mut kinds, seen)?;
    let mut once = BTreeSet::new();
    kinds.retain(|k| once.insert(k.clone()));
    Some(kinds)
}

/// An axis literal over a derived set, rewritten (`derived_axes`): `rr_<helper>_<set>(args)`, its arguments a
/// nearest's two variables, or the axis atom's arguments at `args`.
struct AxisLit {
    node: u32,
    rel: String,
    nearest: Option<(Slot, Slot)>,
    args: Vector<usize>,
    neg: bool,
}

impl ProgramPass for KindSets {
    const NAME: &'static str = "kind sets";
    fn settings(&self) -> Vec<u8> {
        let mut bytes = self.sets.iter().cloned().collect::<Vec<_>>().join("\n").into_bytes();
        if let Some(library) = &self.library {
            // The library's rules, declarations and facts (its digest), and its kind sets, which the digest does not hold.
            bytes.extend_from_slice(&library.digest().to_le_bytes());
            let sets = format!("{:?}", library.kindset_map().unwrap_or_default());
            bytes.extend_from_slice(&crate::content_store::HashKey::sha256(sets.as_bytes()).sha256_bytes().unwrap_or_default());
        }
        bytes
    }
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::front::{Error, axis};
        let input = std::sync::Arc::clone(w.input());
        let v: &Version = &input;
        // The fields' edit, staged (read through as it goes) and stated on the derivation at the end.
        let mut head = crate::code_rule_dsl::logic::program_image::fields::Edit::on(&input);
        let mut kindsets = v.kindset_map()?;
        let was_kindsets = kindsets.clone();
        let library = self.library.clone();
        let library_sets = library.as_ref().map(|l| l.kindset_map()).transpose()?;
        let library = library.as_deref().zip(library_sets.as_ref());
        let mut gone: BTreeSet<String> = BTreeSet::new();
        let mut derived: BTreeSet<String> = BTreeSet::new();
        for set in std::mem::take(&mut self.sets) {
            if kindsets.contains_key(&set) {
                continue;
            }
            // `members`.
            match kinds_of(v, &head, library, &gone, &set, &mut BTreeSet::new()) {
                Some(kinds) => drop(kindsets.insert(set, kinds)),
                None if matches!(head.columns_of(&set).as_deref(), Some([(_, t)]) if t == "sym") => drop(derived.insert(set)),
                None => return Err(Error::Syntax(format!("the kind set {set} is not a relation of kinds (one column of facts, or rules `{set}(k) :- s(k)` over such sets)")).to_string()),
            }
        }
        // A set no rule reads is no relation of the program, to a fixed point.
        let heads_of = |gone: &BTreeSet<String>, set: &str| v.rel(set).is_some_and(|rel| v.heads(rel).next().is_some()) && !gone.contains(set);
        loop {
            let unread: Vec<String> = kindsets
                .keys()
                .filter(|set| head.declares(set) || head.states_facts(set) || heads_of(&gone, set))
                .filter(|set| {
                    !v.rules().filter(|r| !gone.contains(v.rel_name(r.head().rel())) && v.rel_name(r.head().rel()) != set.as_str()).any(|r| {
                        r.body().any(|l| matches!(l.view(), L::Atom(a) | L::Agg { atom: a, .. } if v.rel_name(a.rel()) == set.as_str()))
                    })
                })
                .cloned()
                .collect();
            if unread.is_empty() {
                break;
            }
            for set in unread {
                if head.states_facts(&set) {
                    head.b.fields().drop_facts(&set);
                }
                if head.declares(&set) {
                    head.b.fields().undeclare(&set);
                }
                gone.insert(set);
            }
        }
        // `derived_axes`: each axis over a derived set read through a helper.
        let mut axes: BTreeMap<RuleId, Vec<AxisLit>> = BTreeMap::new();
        let mut need: BTreeSet<(String, &'static str)> = BTreeSet::new();
        if !derived.is_empty() {
            for r in v.rules().filter(|r| !gone.contains(v.rel_name(r.head().rel()))) {
                for l in r.body() {
                    let (set, helper, nearest, args, neg) = match l.view() {
                        L::Nearest { n, a, kindset } if derived.contains(v.text(kindset)) => (v.text(kindset).to_owned(), "near", Some((n, a)), Vector::new(), false),
                        L::Atom(x) => match axis(v.rel_name(x.rel())) {
                            Some((set, has)) if derived.contains(set) => {
                                let xs: Vec<TermRef<'_>> = x.args().collect();
                                let all: Vector<usize> = (0..xs.len()).collect();
                                let (helper, args) = match (has, xs.as_slice()) {
                                    (false, [_, TermRef::Wild]) => ("near", all),
                                    (false, _) => ("anc", all),
                                    (true, [TermRef::Wild, _]) => ("above", rerail_blocks::vector![1]),
                                    (true, _) => ("desc", all),
                                };
                                (set.to_owned(), helper, None, args, x.neg())
                            }
                            _ => continue,
                        },
                        _ => continue,
                    };
                    if helper == "anc" {
                        need.insert((set.clone(), "near"));
                    }
                    need.insert((set.clone(), helper));
                    axes.entry(r.id()).or_default().push(AxisLit { node: l.node(), rel: format!("rr_{helper}_{set}"), nearest, args, neg });
                }
            }
        }
        // `base_relation_derived` for `child` and `node`, the rules gone left out.
        if !need.is_empty() {
            for rel in ["child", "node"] {
                crate::code_rule_dsl::logic::rr::lower::base_relation_derived(&mut head.b.fields(), rel, heads_of(&gone, rel));
            }
        }
        // The helpers' rules, as `derived_axes` pushes them: a set's members once, then each helper's rules.
        type Body = Vec<(String, Vec<&'static str>, bool)>;
        let mut made: Vec<(String, Vec<&'static str>, Body)> = Vec::new();
        for (set, helper) in &need {
            let member = format!("rr_in_{set}");
            if !head.declares(&member) {
                head.b.fields().declare(&member, &[("x", "node")]);
                made.push((member.clone(), vec!["x"], vec![("node".to_owned(), vec!["_", "x", "k", "_", "_"], false), (set.clone(), vec!["k"], false)]));
            }
            let rel = format!("rr_{helper}_{set}");
            let is = |x: &'static str, neg: bool| (member.clone(), vec![x], neg);
            let child = |p: &'static str, c: &'static str| ("child".to_owned(), vec![p, c, "_"], false);
            let me = |args: Vec<&'static str>| (rel.clone(), args, false);
            let near = format!("rr_near_{set}");
            let (cols, bodies): (Vec<&'static str>, Vec<Body>) = match *helper {
                "near" => (vec!["n", "a"], vec![vec![child("a", "n"), is("a", false)], vec![child("p", "n"), is("p", true), me(vec!["p", "a"])]]),
                "anc" => (vec!["n", "a"], vec![vec![(near.clone(), vec!["n", "a"], false)], vec![(near.clone(), vec!["n", "b"], false), me(vec!["b", "a"])]]),
                "desc" => (vec!["n", "a"], vec![vec![is("n", false), child("a", "n")], vec![me(vec!["n", "p"]), child("a", "p")]]),
                _ => (vec!["a"], vec![vec![is("n", false), child("a", "n")], vec![me(vec!["p"]), child("a", "p")]]),
            };
            let pairs: Vec<(&str, &str)> = cols.iter().map(|c| (*c, "node")).collect();
            head.b.fields().declare(&rel, &pairs);
            for body in bodies {
                made.push((rel.clone(), cols.clone(), body));
            }
        }
        if kindsets != was_kindsets {
            head.b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Kindsets(&kindsets));
        }
        if gone.is_empty() && axes.is_empty() && made.is_empty() && head.b.is_empty() {
            return Ok(());
        }
        let b = w.derive()?;
        b.fields().state_fields_of(&head.b);
        use crate::code_rule_dsl::logic::program_image::build::t_of;
        for r in input.rules() {
            if gone.contains(input.rel_name(r.head().rel())) {
                continue;
            }
            let Some(lits) = axes.get(&r.id()) else {
                b.keep(&input, r);
                continue;
            };
            let mut rb = b.rule().source(r.id()).head_from(&input, r, r.head());
            for l in r.body() {
                rb = match lits.iter().find(|x| x.node == l.node()) {
                    None => rb.lit(&input, r, l),
                    Some(x) => {
                        let args: Vec<T<'_>> = match (x.nearest, l.view()) {
                            (Some((n, a)), _) => vec![T::VarAtom(r.var(n)), T::VarAtom(r.var(a))],
                            (None, L::Atom(a)) => {
                                let xs: Vec<TermRef<'_>> = a.args().collect();
                                x.args.iter().map(|&k| t_of(r, xs[k])).collect()
                            }
                            _ => return Err("an axis atom".into()),
                        };
                        rb.atom_with(x.rel.as_str(), &args, x.neg, false)
                    }
                };
            }
            rb.finish()?;
        }
        let term = |a: &'static str| if a == "_" { T::Wild } else { T::Var(a) };
        for (rel, cols, body) in &made {
            let args: Vec<T<'_>> = cols.iter().map(|c| term(c)).collect();
            let mut rb = b.rule().head(rel.as_str(), &args);
            for (r, xs, neg) in body {
                let args: Vec<T<'_>> = xs.iter().map(|c| term(c)).collect();
                rb = rb.atom_with(r.as_str(), &args, *neg, false);
            }
            rb.finish()?;
        }
        Ok(())
    }
}

/// Every constant grammar a rule names, in an `embed` or `embed_value` head or a `parsed` or `sub_tape` atom, is one
/// the language registry has (decision 0082): a supported dialect's label or a fragment. A name it does not have would
/// make no guest and say nothing.
pub struct GrammarNames;

impl ProgramPass for GrammarNames {
    const NAME: &'static str = "grammar names";
    fn run(&mut self, cx: &Cx<'_>, _w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::{grammar_column, no_grammar};
        let v = cx.v();
        for r in v.rules() {
            let body = r.body().filter_map(|l| match l.view() {
                L::Atom(a) => Some(a),
                _ => None,
            });
            for a in std::iter::once(r.head()).chain(body) {
                if let Some(i) = grammar_column(v.rel_name(a.rel()))
                    && let Some(TermRef::Str(g)) = a.args().nth(i)
                    && !crate::code_rule_dsl::language::is_grammar(v.text(g))
                {
                    return Err(no_grammar(v.rel_name(r.head().rel()), v.text(g)).to_string());
                }
            }
        }
        Ok(())
    }
}

/// The built-ins lowering writes out (`nearest`, `within`, …) and the core relations it rewrites where the program
/// does not derive them (`rewritten`, its settings: `next_sibling`, `precedes`) are declared in `core.rerail` for the
/// checker; they are no relations of the program. `within` stays declared where a rule reads it: the lowering made it
/// the engine's relation (decision 0050); a program lowered without `core.rerail` (its own part, `parse_own`) still
/// declares it.
pub struct BuiltinsOut {
    pub rewritten: Vec<String>,
}

impl ProgramPass for BuiltinsOut {
    const NAME: &'static str = "built-ins out";
    fn settings(&self) -> Vec<u8> {
        self.rewritten.join("\n").into_bytes()
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        let v = cx.v();
        let reads_within = v.rules().any(|r| r.body().any(|l| matches!(l.view(), L::Atom(a) if v.rel_name(a.rel()) == WITHIN)));
        let out: Vec<&str> = crate::code_rule_dsl::logic::rr::lower::builtins(&self.rewritten).filter(|b| !(reads_within && *b == "within")).collect();
        let head = w.on()?;
        for &b in &out {
            if head.declares(b) {
                head.fields().undeclare(b);
            }
        }
        if out.iter().any(|b| head.input_names().iter().any(|i| i == b)) {
            head.fields().retain_inputs(|i| !out.contains(&i));
        }
        if reads_within {
            head.fields().declare_absent(WITHIN, &[("n", "node"), ("a", "node")]);
            head.fields().add_input(WITHIN);
        }
        Ok(())
    }
}

/// `lower::span_patterns` on the version: the same analysis, read off the image's rules and header, the rx spans and
/// AST patterns it finds added to the header. `library`: the columns' spans of the library a module is lowered in the
/// scope of (`rr::Library::spans`), with the library version's digest for the pass's key.
pub struct SpanPatterns {
    pub library: Option<(std::sync::Arc<crate::code_rule_dsl::logic::rr::lower::SpanColumns>, u64)>,
}

/// `f` over the version read as `lower::span_patterns` reads a program, the library's columns' spans `library`.
pub fn with_span_program<R>(v: &Version, library: Option<&crate::code_rule_dsl::logic::rr::lower::SpanColumns>, f: impl FnOnce(&crate::code_rule_dsl::logic::rr::lower::SpanProgram<'_>) -> R) -> Result<R, String> {
    use crate::code_rule_dsl::logic::rr::lower::{SpanAtom, SpanProgram, SpanRule, SpanTerm};
    let (inputs, outputs, natives) = (v.input_names(), v.output_names(), v.native_map()?);
    let term = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, t: TermRef<'_>| match t {
        TermRef::Var(x) => SpanTerm::Var(v.text(r.var(x))),
        TermRef::Str(x) => SpanTerm::Str(v.text(x)),
        _ => SpanTerm::Other,
    };
    let program = SpanProgram {
        rules: v
            .rules()
            .map(|r| SpanRule {
                head: SpanAtom { rel: v.rel_name(r.head().rel()), args: r.head().args().map(|t| term(r, t)).collect(), neg: false },
                body: r
                    .body()
                    .filter_map(|l| match l.view() {
                        L::Atom(a) | L::Agg { atom: a, .. } => Some(SpanAtom { rel: v.rel_name(a.rel()), args: a.args().map(|t| term(r, t)).collect(), neg: a.neg() }),
                        _ => None,
                    })
                    .collect(),
            })
            .collect(),
        decls: {
            let mut d: Vec<(&str, usize)> = v.decls().into_iter().map(|d| (v.text(d.name()), d.cols().count())).collect();
            d.sort();
            d
        },
        inputs: inputs.iter().map(String::as_str).collect(),
        natives: natives.keys().map(String::as_str).collect(),
        outputs: outputs.iter().map(String::as_str).collect(),
        library,
    };
    Ok(f(&program))
}

impl ProgramPass for SpanPatterns {
    const NAME: &'static str = "span patterns";
    fn settings(&self) -> Vec<u8> {
        self.library.as_ref().map(|(_, digest)| digest.to_le_bytes().to_vec()).unwrap_or_default()
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::span_patterns;
        let v = cx.v();
        let found = with_span_program(v, self.library.as_ref().map(|(spans, _)| &**spans), span_patterns)?.map_err(|e| e.to_string())?;
        let (was_spans, was_patterns) = (v.rx_spans()?, v.ast_patterns()?);
        let (mut spans, mut patterns) = (was_spans.clone(), was_patterns.clone());
        spans.extend(found.0);
        patterns.extend(found.1);
        if spans != was_spans {
            w.on()?.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::RxSpans(&spans));
        }
        if patterns != was_patterns {
            w.on()?.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::AstPatterns(&patterns));
        }
        Ok(())
    }
}

/// core.rerail's declarations of what the rules read, then `node`'s projections (`lower::core_and_projections`) on the
/// version: the header's declarations and inputs, and the projections' rules written first, where the tree puts them.
/// Its setting: whether the program's `docof` is core.rerail's, over spans, as the lowerer found it.
pub struct CoreDecls {
    pub docof_of_locs: bool,
}

impl ProgramPass for CoreDecls {
    const NAME: &'static str = "core declarations";
    fn settings(&self) -> Vec<u8> {
        vec![u8::from(self.docof_of_locs)]
    }
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::{core::declare_core, projection_at, projections, write_projection};
        let input = std::sync::Arc::clone(w.input());
        let v: &Version = &input;
        // The fields' edit, staged and stated on the edit or the derivation at the end.
        let mut head = crate::code_rule_dsl::logic::program_image::fields::Edit::on(&input);
        let read: BTreeSet<(String, usize)> = v
            .rules()
            .flat_map(|r| r.body())
            .filter_map(|l| match l.view() {
                L::Atom(a) | L::Agg { atom: a, .. } => Some((v.rel_name(a.rel()).to_owned(), a.args().count())),
                _ => None,
            })
            .collect();
        let mut docof = self.docof_of_locs;
        declare_core(&mut head.b.fields(), &read, &mut docof).map_err(|e| e.to_string())?;
        let derives_node = v.rel("node").is_some_and(|rel| v.heads(rel).next().is_some());
        let made = projections(&mut head.b.fields(), &read, docof, derives_node);
        if made.is_empty() {
            if !head.b.is_empty() {
                w.on()?.fields().state_fields_of(&head.b);
            }
            return Ok(());
        }
        // The program's order, the projections inserted where the tree inserts them.
        enum Item<'r> {
            Old(crate::code_rule_dsl::logic::program_image::RuleRef<'r>),
            New(&'static str),
        }
        let mut order: Vec<Item<'_>> = input.rules().map(Item::Old).collect();
        for rel in made {
            let first = order.first().map(|x| match x {
                Item::Old(r) => input.rel_name(r.head().rel()),
                Item::New(r) => r,
            });
            order.insert(projection_at(rel, first), Item::New(rel));
        }
        let b = w.derive()?;
        b.fields().state_fields_of(&head.b);
        for x in order {
            match x {
                Item::Old(r) => drop(b.keep(&input, r)),
                Item::New(rel) => drop(write_projection(b, rel)?),
            }
        }
        Ok(())
    }
}

/// The findings of the `.violation`s (`lower::findings`) on the version: `lower::findings::findings_on` writes them
/// onto a derivation of the version, given what it reads of the rules (`RulesSeen`, read off the image): its fields, and
/// its rules after the program's, whose own `violation` heads of five columns are widened. Where it writes no rules
/// (`findings_write`), its fields' edit alone. Its settings: the pending
/// findings, which the lowerer hands on.
pub struct Findings {
    pub pending: crate::code_rule_dsl::logic::rr::lower::PendingFindings,
    /// The linked library of a module lowered in its scope (`rr::Library::version`): its rules may read the guests a
    /// finding of this module's is at (`RulesSeen::reads_guests`).
    pub library: Option<std::sync::Arc<Version>>,
}

impl ProgramPass for Findings {
    const NAME: &'static str = "findings";
    fn settings(&self) -> Vec<u8> {
        let mut key = self.pending.key();
        if let Some(library) = &self.library {
            key.extend_from_slice(&library.digest().to_le_bytes());
        }
        key
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::{RulesSeen, findings::{findings_on, findings_write}, is_place_call};
        let v = cx.v();
        fn place(v: &Version, t: TermRef<'_>) -> bool {
            match t {
                TermRef::Call(f, args) => is_place_call(v.text(f)) || args.into_iter().any(|a| place(v, a)),
                _ => false,
            }
        }
        let seen = RulesSeen {
            place_values: v.rules().any(|r| {
                r.head().args().any(|t| place(v, t))
                    || r.body().any(|l| match l.view() {
                        L::Atom(a) | L::Agg { atom: a, .. } => a.args().any(|t| place(v, t)),
                        L::Cmp { l, r, .. } => place(v, l) || place(v, r),
                        _ => false,
                    })
            }),
            reads_tool_call: v.rules().flat_map(|r| r.body()).find_map(|l| match l.view() {
                L::Atom(a) => crate::code_rule_dsl::logic::rr::lower::TOOL_CALLS.into_iter().find(|t| v.rel_name(a.rel()) == *t),
                _ => None,
            }),
            heads: v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect(),
            reads_guests: {
                let reads = |v: &Version| {
                    v.rules().flat_map(|r| r.body()).any(|l| match l.view() {
                        L::Atom(a) | L::Agg { atom: a, .. } => super::super::guests::READS.contains(&v.rel_name(a.rel())),
                        _ => false,
                    })
                };
                let declared = |rel: &str| v.declares(rel) || self.library.as_ref().is_some_and(|l| l.declares(rel));
                (reads(v) || self.library.as_deref().is_some_and(reads)) && declared("guest_place") && declared("tape_of")
            },
        };
        let input = std::sync::Arc::clone(w.input());
        if !findings_write(&*input, &self.pending) {
            // No rules: the fields' edit alone (`guest_policy`'s facts).
            let mut head = crate::code_rule_dsl::logic::program_image::fields::Edit::on(&input);
            findings_on(&mut head.b, &self.pending, seen).map_err(|e| e.to_string())?;
            if !head.b.is_empty() {
                w.on()?.fields().state_fields_of(&head.b);
            }
            return Ok(());
        }
        // The program's rules, its own `violation` heads of five columns widened, then the findings' (`findings_on`).
        let five = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| v.rel_name(r.head().rel()) == "violation" && r.head().args().count() == 5;
        let b = w.derive()?;
        use crate::code_rule_dsl::logic::program_image::build::t_of;
        for r in input.rules() {
            if !five(r) {
                b.keep(&input, r);
                continue;
            }
            let mut args: Vec<T<'_>> = r.head().args().map(|t| t_of(r, t)).collect();
            args.push(T::Int(0));
            let mut rb = b.rule().source(r.id()).head("violation", &args);
            for l in r.body() {
                rb = rb.lit(&input, r, l);
            }
            rb.finish()?;
        }
        findings_on(b, &self.pending, seen).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// `.decl r(…) must, may` (core.rerail 3): r is computed twice. Its may side, which keeps the name, holds what is proven
/// or unknown: every rule of r. Its must side (`rr_must_r`) holds what is proven: the rules with no prefix or `must`.
/// A relation that reads a three-valued one is three-valued too (`lower::three_closure`), with every rule on both
/// sides. In a must side's rule a positive atom of a three-valued relation reads its must side and a negated one its
/// may side; a may side's rule reads the other way (`lower::reads_must`). The must sides are declared and given the
/// facts. A finding is the may side: a definite one and a possible one have the same key, and the possible tag is not
/// made yet. Its settings: the relations declared
/// `must, may`, and the rules' prefixes by index (the version's order is the tree's).
pub struct ThreeValued {
    pub three: BTreeSet<String>,
    pub modal: BTreeMap<usize, String>,
}

/// `ThreeValued`'s rewrite of one side.
struct Side<'a> {
    v: &'a Version,
    three: &'a BTreeSet<String>,
    must: bool,
}

impl crate::code_rule_dsl::logic::program_image::build::Rewrite for Side<'_> {
    fn atom_as(&mut self, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>) -> Option<(String, bool, bool)> {
        let rel = self.v.rel_name(a.rel());
        crate::code_rule_dsl::logic::rr::lower::reads_must(self.three, rel, a.neg(), self.must).then(|| (crate::code_rule_dsl::logic::rr::lower::must_name(rel), a.neg(), a.gated()))
    }
    fn head_as(&mut self, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>) -> Option<String> {
        self.must.then(|| crate::code_rule_dsl::logic::rr::lower::must_name(self.v.rel_name(a.rel())))
    }
}

impl ProgramPass for ThreeValued {
    const NAME: &'static str = "three-valued relations";
    fn settings(&self) -> Vec<u8> {
        format!("{:?} {:?}", self.three, self.modal).into_bytes()
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::rr::lower::{three_closure, three_valued_header};
        if self.three.is_empty() {
            return Ok(());
        }
        let v = cx.v();
        let shape: Vec<(&str, Vec<&str>)> = v
            .rules()
            .map(|r| {
                let reads = r
                    .body()
                    .filter_map(|l| match l.view() {
                        L::Atom(a) | L::Agg { atom: a, .. } => Some(v.rel_name(a.rel())),
                        _ => None,
                    })
                    .collect();
                (v.rel_name(r.head().rel()), reads)
            })
            .collect();
        let three = three_closure(&self.three, &shape);
        drop(shape);
        let input = std::sync::Arc::clone(w.input());
        let b = w.derive()?;
        three_valued_header(&mut b.fields(), &three);
        for (i, r) in input.rules().enumerate() {
            if !three.contains(input.rel_name(r.head().rel())) {
                b.keep(&input, r);
                continue;
            }
            if self.modal.get(&i).map(String::as_str) != Some("may") {
                b.rewrite(&input, r, &mut Side { v: &input, three: &three, must: true }).finish()?;
            }
            b.rewrite(&input, r, &mut Side { v: &input, three: &three, must: false }).finish()?;
        }
        Ok(())
    }
}

/// A program to run reads only relations it declares or the transducer makes: else an error, as in Soufflé, not a
/// plan's panic. (Parsing alone does not ask it: a parse's tests read relations they never declare.) Every relation
/// a rule reads is declared (`front::read_declared`), and each atom has the columns its relation declares
/// (`front::columns_declared`). `match_all` and `match_group` over a span are built-ins: the text columns make each
/// regex's relation (`strings`).
pub struct Declared;

impl ProgramPass for Declared {
    const NAME: &'static str = "declared";
    fn run(&mut self, cx: &Cx<'_>, _w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::front::{columns_declared, read_declared};
        let v = cx.v();
        let head: &Version = v;
        for r in v.rules() {
            let name = v.rel_name(r.head().rel());
            let atoms = || {
                r.body().filter_map(|l| match l.view() {
                    L::Atom(a) | L::Agg { atom: a, .. } => Some(a),
                    _ => None,
                })
            };
            for a in atoms() {
                read_declared(head, v.rel_name(a.rel()), name).map_err(|e| e.to_string())?;
            }
            for a in std::iter::once(r.head()).chain(atoms()) {
                columns_declared(head, v.rel_name(a.rel()), a.args().count(), name).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
