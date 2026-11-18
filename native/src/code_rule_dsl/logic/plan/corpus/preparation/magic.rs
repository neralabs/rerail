//! The program's own demand on the version (decision 0281's path, step 6d): `magic::demand_bound_columns`, a
//! corpus run's `.pragma "magic-transform" "r, …"` (decision 0145), ported onto the version's rules. It is
//! `magic_adorn` with every relation not listed computed in full, with the options the preparation runs it with
//! (`magic::Options::default`: no gates, every column adorned, inputs first sideways).
//!
//! Each rule's literals are its own handles; what the transformation adds to a rule is its guard
//! (`magic_<r>_<ad>(…)`), a computed head term's binding (`_m<n> = @f(…)`) and `magic_true(0)`. The variables are a
//! rule's slots, a binding's fresh variable after them. Each program the trials make is a dependency graph, not a
//! program: what `front::strata` reads of one.

use crate::code_rule_dsl::logic::front::{Edges, Error};
use crate::code_rule_dsl::logic::front::ast::Report;
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, AggRef, AtomRef, Cell, CmpOp, Lit, LitRef, RuleBuilder, RuleRef, Slot, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};

/// A term of a literal the transformation writes: the rule's own, or the fresh variable `_m<n>`.
#[derive(Clone, Copy)]
enum Tm<'v> {
    Own(TermRef<'v>),
    Fresh(u32),
}

/// A literal of a rule as the transformation reads it.
#[derive(Clone)]
enum M<'v> {
    Own(LitRef<'v>),
    /// A positive atom (a guard): its relation and its terms.
    Guard(String, Vec<Tm<'v>>),
    /// `_m<n> = call`.
    Call(u32, TermRef<'v>),
    /// `magic_true(0)`.
    True,
}

/// The rules of the program being made, as a trial writes them: magic rules, adorned rules and views.
#[derive(Clone)]
struct Made<'v> {
    rule: RuleRef<'v>,
    head: Option<(String, Vec<Tm<'v>>)>,
    body: Vec<M<'v>>,
}

struct Ctx<'v, 'a> {
    v: &'v Version,
    about: &'a About,
    by: &'a BTreeMap<&'v str, Vec<RuleRef<'v>>>,
    idb: &'a BTreeSet<&'v str>,
    level: &'a BTreeMap<String, usize>,
    /// `answers`' results for the adornments asked at the top level, per `closure`.
    answered: BTreeMap<(String, String), bool>,
}

fn fresh(r: RuleRef<'_>, n: u32) -> Slot {
    Slot(r.var_count() as u32 + n)
}

fn tm_vars(r: RuleRef<'_>, t: Tm<'_>, out: &mut VarSet) {
    match t {
        Tm::Own(t) => term_vars(t, out),
        Tm::Fresh(n) => {
            out.insert(fresh(r, n));
        }
    }
}

fn is_const(t: Tm<'_>) -> bool {
    matches!(t, Tm::Own(TermRef::Int(_) | TermRef::Str(_)))
}

fn bset(ad: &str) -> BTreeSet<usize> {
    ad.bytes().enumerate().filter(|(_, x)| *x == b'b').map(|(i, _)| i).collect()
}

impl<'v> M<'v> {
    fn vars(&self, r: RuleRef<'v>) -> VarSet {
        let mut out = VarSet::new();
        match self {
            M::Own(l) => return lit_vars(*l),
            M::Guard(_, args) => args.iter().for_each(|t| tm_vars(r, *t, &mut out)),
            M::Call(n, call) => {
                out.insert(fresh(r, *n));
                term_vars(*call, &mut out);
            }
            M::True => {}
        }
        out
    }
    /// A positive atom (`magic::positive`): its relation.
    fn positive(&self, v: &'v Version) -> Option<&str> {
        match self {
            M::Own(l) => match l.view() {
                Lit::Atom(a) if !a.neg() => Some(v.rel_name(a.rel())),
                _ => None,
            },
            M::Guard(rel, _) => Some(rel),
            M::True => Some("magic_true"),
            M::Call(..) => None,
        }
    }
    /// The first term of a positive atom.
    fn first(&self) -> Option<Tm<'v>> {
        match self {
            M::Own(l) => l.atom().and_then(|a| a.args().next()).map(Tm::Own),
            M::Guard(_, args) => args.first().copied(),
            M::True => None,
            M::Call(..) => None,
        }
    }
    /// The atom it reads (`front::Literal::atom`): its relation, whether negated or aggregated.
    fn atom(&self, v: &'v Version) -> Option<(&str, bool, bool)> {
        match self {
            M::Own(l) => match l.view() {
                Lit::Atom(a) => Some((v.rel_name(a.rel()), a.neg(), false)),
                Lit::Agg { atom, .. } => Some((v.rel_name(atom.rel()), atom.neg(), true)),
                _ => None,
            },
            M::Guard(rel, _) => Some((rel, false, false)),
            M::True => Some(("magic_true", false, false)),
            M::Call(..) => None,
        }
    }
    fn own_atom(&self) -> Option<AtomRef<'v>> {
        match self {
            M::Own(l) => l.atom(),
            _ => None,
        }
    }
    fn is_agg(&self) -> bool {
        matches!(self, M::Own(l) if matches!(l.view(), Lit::Agg { .. }))
    }
    fn gated(&self) -> bool {
        match self {
            M::Own(l) => l.atom().is_some_and(|a| a.gated()),
            _ => false,
        }
    }
    fn show(&self, v: &Version, r: RuleRef<'_>) -> String {
        match self {
            M::Own(l) => v.show_lit(r, *l),
            M::Guard(rel, args) => format!("{rel}({})", args.iter().map(|t| show_tm(v, r, *t)).collect::<Vec<_>>().join(", ")),
            M::Call(n, call) => format!("$_m{n} = {}", v.show_term(r, *call)),
            M::True => "magic_true(0)".into(),
        }
    }
}

fn show_tm(v: &Version, r: RuleRef<'_>, t: Tm<'_>) -> String {
    match t {
        Tm::Own(t) => v.show_term(r, t),
        Tm::Fresh(n) => format!("$_m{n}"),
    }
}

fn is_cat(v: &Version, t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Call(f, args) if v.text(f) == "cat" && { args.clone().all(|a| matches!(a, TermRef::Var(_))) })
}

/// A comparison's variables once it runs (`front::cmp_binds`), its left side a term or a fresh variable.
fn cmp_binds(v: &Version, r: RuleRef<'_>, l: Tm<'_>, op: CmpOp, rt: TermRef<'_>, bound: &VarSet) -> Option<VarSet> {
    let mut rv = VarSet::new();
    term_vars(rt, &mut rv);
    let lvar = match l {
        Tm::Own(TermRef::Var(x)) => Some(x),
        Tm::Fresh(n) => Some(fresh(r, n)),
        _ => None,
    };
    if op == CmpOp::Eq
        && let Some(x) = lvar
        && !bound.contains(x)
    {
        return rv.is_subset(bound).then(|| VarSet::of([x]));
    }
    let mut all = rv.clone();
    tm_vars(r, l, &mut all);
    if all.is_subset(bound) {
        return Some(VarSet::new());
    }
    if op == CmpOp::Eq && lvar.is_some() && is_cat(v, rt) {
        return Some(rv.minus(bound));
    }
    None
}

/// The variables literal `l` binds once it runs, or `None` if it cannot run yet (`magic::binds`).
fn binds(v: &Version, r: RuleRef<'_>, l: &M<'_>, bound: &VarSet) -> Option<VarSet> {
    match l {
        M::Own(x) => match x.view() {
            Lit::Atom(a) if !a.neg() => Some(atom_vars(a)),
            Lit::Atom(a) => atom_vars(a).is_subset(bound).then(VarSet::new),
            Lit::Cmp { l, op, r: rt } => cmp_binds(v, r, Tm::Own(l), op, rt, bound),
            Lit::Agg { var, op, over, atom } => {
                let mut need = atom_vars(atom);
                need.remove(var);
                if let Some(o) = over {
                    need.remove(o);
                }
                if let AggRef::Join { by, .. } = op {
                    by.for_each(|k| need.remove(k));
                }
                need.is_subset(bound).then(|| VarSet::of([var]))
            }
            Lit::Nearest { n, a, .. } => (bound.contains(n) || bound.contains(a)).then(|| lit_vars(*x)),
            Lit::Ast { .. } => Some(lit_vars(*x)),
        },
        M::Guard(..) | M::True => Some(l.vars(r)),
        M::Call(n, call) => cmp_binds(v, r, Tm::Fresh(*n), CmpOp::Eq, *call, bound),
    }
}

/// An adorned rule's guard and its computed head terms' bindings, for adornment `ad` of rule `r`'s head.
fn guard<'v>(r: RuleRef<'v>, rel: &str, ad: &str) -> (M<'v>, Vec<M<'v>>) {
    let (mut gargs, mut calls) = (Vec::new(), Vec::new());
    for (t, x) in r.head().args().zip(ad.bytes()) {
        if x != b'b' {
            continue;
        }
        if let TermRef::Call(..) = t {
            let n = calls.len() as u32;
            calls.push(M::Call(n, t));
            gargs.push(Tm::Fresh(n));
        } else {
            gargs.push(Tm::Own(t));
        }
    }
    (M::Guard(format!("magic_{rel}_{ad}"), gargs), calls)
}

type Magic<'v> = Vec<((String, Vec<Tm<'v>>), RuleRef<'v>, Vec<(M<'v>, bool)>)>;

/// A failure of one attempt: a negative cycle demand closed (the relations negated or aggregated inside it), or any
/// other error.
enum Attempt {
    Cycle { error: Error, negs: BTreeSet<String> },
    Failed(Error),
}

impl From<Error> for Attempt {
    fn from(e: Error) -> Self {
        Attempt::Failed(e)
    }
}

impl<'v> Ctx<'v, '_> {
    fn arity(&self, rel: &str) -> usize {
        self.v.decl_of(rel).map_or(0, |d| d.arity())
    }
    /// A relation's key columns: all but a lattice's value.
    fn keyed(&self, rel: &str) -> usize {
        self.about.lattice.iter().find(|l| self.v.rel_name(l.rel) == rel).map_or_else(|| self.arity(rel), |l| l.keys)
    }
    fn pattern(&self, a: AtomRef<'_>, bound: &VarSet) -> String {
        let keyed = self.keyed(self.v.rel_name(a.rel()));
        a.args()
            .enumerate()
            .map(|(i, t)| {
                let b = i < keyed && (matches!(t, TermRef::Int(_) | TermRef::Str(_)) || matches!(t, TermRef::Var(x) if bound.contains(x)));
                if b { 'b' } else { 'f' }
            })
            .collect()
    }
    fn var_bound(a: AtomRef<'_>, ad: &str) -> bool {
        a.args().zip(ad.bytes()).any(|(t, x)| x == b'b' && matches!(t, TermRef::Var(_)))
    }
    fn adorn(&mut self, a: AtomRef<'_>, bound: &VarSet, full: &BTreeSet<String>) -> String {
        let ad = self.pattern(a, bound);
        if !ad.starts_with('b') {
            let free = "f".repeat(ad.len());
            return if Self::var_bound(a, &ad) && self.answers(self.v.rel_name(a.rel()), &ad, &[], full) { ad } else { free };
        }
        ad
    }
    /// Whether R's rules answer the non-key adornment `ad` without asking any derived relation in full, or through a
    /// cycle of non-key adornments (`magic::Adorn::answers`).
    fn answers(&mut self, rel: &str, ad: &str, stack: &[(String, String)], full: &BTreeSet<String>) -> bool {
        let key = (rel.to_owned(), ad.to_owned());
        if let Some(&ok) = self.answered.get(&key) {
            return ok;
        }
        if full.contains(rel) || stack.contains(&key) {
            return false;
        }
        let mut ok = true;
        let v = self.v;
        'rules: for &rule in self.by.get(rel).into_iter().flatten() {
            let mut bound = VarSet::new();
            let mut extra = Vec::new();
            for (t, x) in rule.head().args().zip(ad.bytes()) {
                if x != b'b' {
                    continue;
                }
                match t {
                    TermRef::Call(..) => {
                        let n = extra.len() as u32;
                        extra.push(M::Call(n, t));
                        bound.insert(fresh(rule, n));
                    }
                    TermRef::Var(s) => {
                        bound.insert(s);
                    }
                    _ => {}
                }
            }
            let mut body: Vec<M<'_>> = rule.body().map(M::Own).collect();
            body.extend(extra);
            let Some(order) = self.sips(rule, &body, &bound) else {
                ok = false;
                break;
            };
            for l in &order {
                if let M::Own(x) = l
                    && let Lit::Atom(a) = x.view()
                    && self.idb.contains(v.rel_name(a.rel()))
                    && !full.contains(v.rel_name(a.rel()))
                {
                    let ad2 = self.pattern(a, &bound);
                    if !ad2.starts_with('b') {
                        let mut deeper = stack.to_vec();
                        deeper.push(key.clone());
                        if !(Self::var_bound(a, &ad2) && self.answers(v.rel_name(a.rel()), &ad2, &deeper, full)) {
                            ok = false;
                            break 'rules;
                        }
                    }
                }
                let b = binds(v, rule, l, &bound).unwrap_or_default();
                bound.extend(&b);
            }
        }
        if stack.is_empty() {
            self.answered.insert(key, ok);
        }
        ok
    }
    /// The body in sideways order (`magic::Adorn::sips`).
    fn sips(&self, r: RuleRef<'v>, body: &[M<'v>], bound: &VarSet) -> Option<Vec<M<'v>>> {
        let v = self.v;
        let (mut rest, mut order, mut bound) = (body.to_vec(), Vec::new(), bound.clone());
        while !rest.is_empty() {
            if let Some(i) = rest.iter().position(|l| l.positive(v).is_none() && binds(v, r, l, &bound).is_some()) {
                let l = rest.remove(i);
                let b = binds(v, r, &l, &bound).unwrap_or_default();
                bound.extend(&b);
                order.push(l);
                continue;
            }
            let pos: Vec<usize> = (0..rest.len()).filter(|&i| rest[i].positive(v).is_some()).collect();
            if pos.is_empty() {
                return None;
            }
            let first = |i: usize| match rest[i].first() {
                Some(t) if is_const(t) => true,
                Some(Tm::Own(TermRef::Var(x))) => bound.contains(x),
                Some(Tm::Fresh(n)) => bound.contains(fresh(r, n)),
                _ => false,
            };
            let derived = |i: usize| self.idb.contains(rest[i].positive(v).expect("a positive atom"));
            let joined: Vec<usize> = pos.iter().copied().filter(|&i| !rest[i].vars(r).is_disjoint(&bound)).collect();
            let lowest = |ix: &[usize], key: &dyn Fn(usize) -> i64| {
                ix.iter().copied().fold(None, |best: Option<usize>, i| match best {
                    Some(b) if key(b) <= key(i) => Some(b),
                    _ => Some(i),
                })
            };
            let pick = pos
                .iter()
                .copied()
                .find(|&i| first(i) && !derived(i))
                .or_else(|| pos.iter().copied().find(|&i| first(i)))
                .or_else(|| lowest(&joined, &|i| i64::from(derived(i))))
                .or_else(|| lowest(&pos, &|i| self.level.get(rest[i].positive(v).expect("a positive atom")).map_or(-1, |&l| l as i64)))
                .expect("a positive atom");
            let l = rest.remove(pick);
            bound.extend(&l.vars(r));
            order.push(l);
        }
        Some(order)
    }
    /// The adorned rules and magic rules reachable from the outputs (all free), and the adornments seen.
    fn closure(&mut self, full: &BTreeSet<String>, coarse: &BTreeMap<(String, String), String>) -> Result<(Vec<Made<'v>>, Magic<'v>, BTreeSet<(String, String)>), Error> {
        let v = self.v;
        let (mut rules, mut magic, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
        self.answered.clear();
        let mut todo: Vec<(String, String)> = v.outputs().map(|o| v.text(o)).filter(|o| self.idb.contains(o)).map(|o| (o.to_owned(), "f".repeat(self.arity(o)))).collect();
        while let Some((rel, mut ad)) = todo.pop() {
            if full.contains(&rel) {
                ad = "f".repeat(self.arity(&rel));
            }
            if !seen.insert((rel.clone(), ad.clone())) {
                continue;
            }
            for rule in self.by.get(rel.as_str()).cloned().unwrap_or_default() {
                let (mut bound, mut pre, mut gd) = (VarSet::new(), Vec::<(M<'_>, bool)>::new(), None);
                let mut body: Vec<M<'_>> = rule.body().map(M::Own).collect();
                if ad.contains('b') {
                    let (g, calls) = guard(rule, &rel, &ad);
                    body.extend(calls);
                    bound.extend(&g.vars(rule));
                    pre.push((g.clone(), false));
                    gd = Some(g);
                }
                let order = self.sips(rule, &body, &bound).ok_or_else(|| Error::Syntax(format!("no order binds the body of {}", v.show_rule(rule))))?;
                for l in order {
                    if let Some(a) = l.own_atom()
                        && self.idb.contains(v.rel_name(a.rel()))
                    {
                        let arel = v.rel_name(a.rel());
                        let ad2 = if full.contains(arel) || l.is_agg() { "f".repeat(self.arity(arel)) } else { self.adorn(a, &bound, full) };
                        let ad2 = coarse.get(&(arel.to_owned(), ad2.clone())).cloned().unwrap_or(ad2);
                        if ad2.contains('b') {
                            let args = a.args().zip(ad2.bytes()).filter(|(_, x)| *x == b'b').map(|(t, _)| Tm::Own(t)).collect();
                            magic.push(((format!("magic_{arel}_{ad2}"), args), rule, pre.clone()));
                        }
                        todo.push((arel.to_owned(), ad2));
                    }
                    match &l {
                        M::Own(x) if matches!(x.view(), Lit::Atom(a) if !a.neg()) => {
                            let extra = l.positive(v).is_some_and(|rel| self.idb.contains(rel));
                            pre.push((l.clone(), extra));
                        }
                        M::Own(x) if matches!(x.view(), Lit::Agg { .. }) => {}
                        M::Own(x) => pre.push((l.clone(), matches!(x.view(), Lit::Atom(_)))),
                        _ => pre.push((l.clone(), false)),
                    }
                    let b = binds(v, rule, &l, &bound).unwrap_or_default();
                    bound.extend(&b);
                }
                let mut out = Vec::new();
                out.extend(gd);
                out.extend(body);
                rules.push(Made { rule, head: None, body: out });
            }
        }
        Ok((rules, magic, seen))
    }
}

/// A trial's program's dependency graph, stratified (`front::strata`): its components, dependencies first, and its
/// edges by name.
fn strata(v: &Version, about: &About, rules: &[Made<'_>]) -> Result<(Vec<BTreeSet<String>>, Edges), Error> {
    let head_of = |m: &Made<'_>| -> String { m.head.as_ref().map_or_else(|| v.rel_name(m.rule.head().rel()).to_owned(), |(rel, _)| rel.clone()) };
    let heads: Vec<String> = rules.iter().map(head_of).collect();
    let mut names: Vec<&str> = heads.iter().map(String::as_str).collect();
    for (_, p) in &about.answers {
        names.push(v.rel_name(*p));
    }
    names.sort_unstable();
    names.dedup();
    let id: rustc_hash::FxHashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (*n, i as u32)).collect();
    let mut succ: Vec<Vec<(u32, bool)>> = vec![Vec::new(); names.len()];
    let mut edges: Edges = BTreeMap::new();
    for (q, p) in &about.answers {
        if let Some(&r) = id.get(v.rel_name(*q)) {
            succ[id[v.rel_name(*p)] as usize].push((r, false));
            edges.entry(v.rel_name(*p).to_owned()).or_default().insert((v.rel_name(*q).to_owned(), false));
        }
    }
    for n in &about.natives {
        let Some(&h) = id.get(v.rel_name(n.rel)) else { continue };
        for &read in &n.reads {
            if let Some(&r) = id.get(v.rel_name(read)) {
                succ[h as usize].push((r, false));
                edges.entry(v.rel_name(n.rel).to_owned()).or_default().insert((v.rel_name(read).to_owned(), false));
            }
        }
    }
    let mut aggs = rustc_hash::FxHashSet::default();
    for (m, head) in rules.iter().zip(&heads) {
        let h = id[head.as_str()];
        let lattice = about.lattice.iter().find(|l| v.rel_name(l.rel) == head.as_str());
        for l in &m.body {
            let Some((rel, neg, is_agg)) = l.atom(v) else { continue };
            let Some(&d) = id.get(rel) else { continue };
            let agg = is_agg
                && !lattice.is_some_and(|d| {
                    let op = match l {
                        M::Own(x) => match x.view() {
                            Lit::Agg { op, .. } => Some(op),
                            _ => None,
                        },
                        _ => None,
                    };
                    matches!((op, v.text(d.lattice)), (Some(AggRef::Count | AggRef::Max), "max") | (Some(AggRef::Min), "min"))
                });
            let gated = l.gated();
            if agg && !gated {
                aggs.insert((h, d));
            }
            succ[h as usize].push((d, (neg || agg) && !gated));
            edges.entry(head.clone()).or_default().insert((rel.to_owned(), (neg || agg) && !gated));
        }
    }
    for s in &mut succ {
        s.sort_unstable();
        s.dedup();
    }
    let sccs = crate::code_rule_dsl::logic::front::components_of_graph(names, succ, aggs)?;
    Ok((sccs, edges))
}

/// The program's own demand: the relations listed (`.pragma "magic-transform"`), adorned over every column their
/// readers bind. Its report, and the rules it wrote; `None` where the program lists none.
#[derive(Default)]
pub struct ProgramDemand {
    /// The relations listed, and what became of them: the transformation's report, or why it was not applied (the
    /// program is then as it was, every relation computed in full).
    pub listed: Vec<String>,
    pub outcome: Option<std::result::Result<Report, String>>,
}

impl ProgramPass for ProgramDemand {
    const NAME: &'static str = "the program's demand";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> std::result::Result<(), String> {
        match self.apply(cx, w) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.outcome = Some(Err(e));
                Ok(())
            }
        }
    }
}

impl ProgramDemand {
    fn apply(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> std::result::Result<(), String> {
        let v = cx.v();
        let about = About::of(v);
        let listed: Vec<&str> = about.demand_list.iter().map(|a| v.text(*a)).collect();
        if listed.is_empty() {
            return Ok(());
        }
        self.listed = listed.iter().map(|x| (*x).to_owned()).collect();
        let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let mut full: BTreeSet<String> = BTreeSet::new();
        if !listed.contains(&"*") {
            full.extend(derived.iter().filter(|r| !listed.contains(r)).map(|r| (*r).to_owned()));
        }
        let mut by: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
        for r in v.rules() {
            by.entry(v.rel_name(r.head().rel())).or_default().push(r);
        }
        let view: BTreeSet<&str> = by
            .iter()
            .filter(|(_, rs)| {
                rs.iter().all(|r| {
                    let body: Vec<LitRef<'_>> = r.body().collect();
                    body.len() == 1 && matches!(body[0].view(), Lit::Atom(a) if !a.neg() && !derived.contains(v.rel_name(a.rel())))
                })
            })
            .map(|(r, _)| *r)
            .collect();
        let idb: BTreeSet<&str> = derived.difference(&view).copied().collect();
        // A relation that aggregates is computed in full.
        full.extend(v.rules().filter(|r| r.body().any(|l| matches!(l.view(), Lit::Agg { .. }))).map(|r| v.rel_name(r.head().rel()).to_owned()));
        let originals: Vec<Made<'_>> = v.rules().map(|rule| Made { rule, head: None, body: rule.body().map(M::Own).collect() }).collect();
        let (sccs, _) = strata(v, &about, &originals).map_err(|e| e.to_string())?;
        let level: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.clone(), i))).collect();
        let mut cyc = Vec::new();
        let made = loop {
            let mut cx2 = Ctx { v, about: &about, by: &by, idb: &idb, level: &level, answered: BTreeMap::new() };
            match attempt(&mut cx2, &full) {
                Ok(made) => break made,
                Err(Attempt::Failed(e)) => return Err(e.to_string()),
                Err(Attempt::Cycle { error, negs }) => {
                    let mut new = BTreeSet::new();
                    let mut todo: Vec<String> = negs.iter().filter(|r| idb.contains(r.as_str()) && !full.contains(*r)).cloned().collect();
                    while let Some(r) = todo.pop() {
                        if new.contains(&r) || full.contains(&r) || !idb.contains(r.as_str()) {
                            continue;
                        }
                        todo.extend(by[r.as_str()].iter().flat_map(|rule| rule.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()));
                        new.insert(r);
                    }
                    if new.is_empty() {
                        return Err(error.to_string());
                    }
                    cyc.extend(negs.intersection(&new).cloned());
                    full.extend(new);
                }
            }
        };
        let (rules, decls, mut report) = made;
        report.full_for_negation = cyc;
        // The program as the attempt made it: its rules, the guards' declarations, `magic_true`, and no demand left.
        let input = w.input().clone();
        let b = w.derive()?;
        for m in &rules {
            write(&input, b.rule(), m)?;
        }
        for (rel, cols) in &decls {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(rel, &cols);
        }
        b.declare("magic_true", &[("k", "int")]);
        let mut rows: Vec<Vec<Cell>> = input.find("magic_true").and_then(|a| input.facts(a)).map(|f| f.rows().map(|r| r.collect()).collect()).unwrap_or_default();
        rows.push(vec![Cell::Int(0)]);
        b.set_fact_cells("magic_true", rows);
        // No demand left, and no gates: a trial's program has none (`magic::Adorn::run`'s `settle`).
        for name in ["demand", "gate", "gate_src", "gate_quiet", "gate_dep"] {
            if !b.about_rows(name).is_empty() {
                b.set_about_rows(name, Vec::new());
            }
        }
        // The report, as the program keeps it (`Program::magic_report`: `magic_report` and `report_list`).
        let pc = super::demand::about_p(b);
        b.set_about_rows("magic_report", vec![vec![pc, Cell::Int(report.magic_rules as i64), Cell::Int(report.extras_kept as i64), Cell::Int(report.gates as i64)]]);
        let mut list = Vec::new();
        for (name, items) in [("adornments", &report.adornments), ("full", &report.full), ("full_for_negation", &report.full_for_negation)] {
            for (k, item) in items.iter().enumerate() {
                let (n, x) = (b.atom(name), b.atom(item));
                list.push(vec![pc, Cell::Str(n), Cell::Int(k as i64), Cell::Str(x)]);
            }
        }
        b.set_about_rows("report_list", list);
        self.outcome = Some(Ok(report));
        Ok(())
    }
}

/// One attempt with `full` computed in full (`magic::Adorn::run`): the rules, the guards' declarations and the
/// report.
#[allow(clippy::type_complexity)]
fn attempt<'v>(cx: &mut Ctx<'v, '_>, full_in: &BTreeSet<String>) -> Result<(Vec<Made<'v>>, Vec<(String, Vec<(String, String)>)>, Report), Attempt> {
    let v = cx.v;
    let (mut full, mut coarse) = (full_in.clone(), BTreeMap::<(String, String), String>::new());
    let (rules, magic, seen) = loop {
        let (rules, magic, seen) = cx.closure(&full, &coarse)?;
        let free: BTreeSet<String> = seen.iter().filter(|(_, a)| !a.contains('b')).map(|(r, _)| r.clone()).filter(|r| !full.contains(r)).collect();
        let mut more = BTreeMap::new();
        for (rel, ad) in &seen {
            let under = seen
                .iter()
                .filter(|(r2, a2)| r2 == rel && a2 != ad && a2.contains('b') && bset(a2).is_subset(&bset(ad)) && bset(a2) != bset(ad))
                .map(|(_, a2)| a2.clone())
                .min_by_key(|a2| (bset(a2).len(), a2.clone()));
            if let Some(under) = under
                && ad.contains('b')
            {
                more.insert((rel.clone(), ad.clone()), under);
            }
        }
        if free.is_empty() && more.is_empty() {
            break (rules, magic, seen);
        }
        full.extend(free);
        coarse.extend(more);
    };
    let mut decls = Vec::new();
    for (rel, ad) in &seen {
        if ad.contains('b') {
            let d = v.decl_of(rel).ok_or_else(|| Error::Syntax(format!("{rel} is not declared")))?;
            let cols = d.cols().zip(ad.bytes()).filter(|(_, x)| *x == b'b').map(|((n, t), _)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
            decls.push((format!("magic_{rel}_{ad}"), cols));
        }
    }
    let mut keep: Vec<Vec<bool>> = magic.iter().map(|(_, _, pre)| pre.iter().map(|(_, extra)| !extra).collect()).collect();
    let views: Vec<Made<'_>> = v.rules().filter(|r| !cx.idb.contains(v.rel_name(r.head().rel()))).map(|rule| Made { rule, head: None, body: rule.body().map(M::Own).collect() }).collect();
    // One magic rule's body, as `ks` keeps its literals.
    let one = |mh: &(String, Vec<Tm<'v>>), r: RuleRef<'v>, pre: &[(M<'v>, bool)], ks: &[bool]| -> Result<Vec<M<'v>>, Error> {
        let mut key = VarSet::new();
        mh.1.iter().for_each(|t| tm_vars(r, *t, &mut key));
        let (mut ks, mut need) = (ks.to_vec(), key.clone());
        for i in (0..pre.len()).rev() {
            let l = &pre[i].0;
            // What `l` gives, and the right side of a comparison (its own or a binding's).
            let (gives, cmp_right): (VarSet, Option<(Option<Slot>, TermRef<'_>)>) = match l {
                M::Own(x) => match x.view() {
                    Lit::Atom(a) if !a.neg() => (lit_vars(*x), None),
                    Lit::Nearest { .. } => (lit_vars(*x), None),
                    Lit::Cmp { l: TermRef::Var(s), op: CmpOp::Eq, r: rt } if is_cat(v, rt) => (lit_vars(*x), Some((Some(s), rt))),
                    Lit::Cmp { l: TermRef::Var(s), op: CmpOp::Eq, r: rt } => (VarSet::of([s]), Some((Some(s), rt))),
                    Lit::Cmp { l: TermRef::Var(s), r: rt, .. } => (VarSet::new(), Some((Some(s), rt))),
                    Lit::Cmp { r: rt, .. } => (VarSet::new(), Some((None, rt))),
                    _ => (VarSet::new(), None),
                },
                M::Guard(..) | M::True => (l.vars(r), None),
                M::Call(n, call) if is_cat(v, *call) => (l.vars(r), Some((Some(fresh(r, *n)), *call))),
                M::Call(n, call) => (VarSet::of([fresh(r, *n)]), Some((Some(fresh(r, *n)), *call))),
            };
            if gives.is_disjoint(&need) {
                continue;
            }
            ks[i] = true;
            // A split gives its parts from the whole, and the whole from its parts.
            let split = match cmp_right {
                Some((Some(s), rt)) if is_cat(v, rt) && {
                    let mut rv = VarSet::new();
                    term_vars(rt, &mut rv);
                    !rv.is_disjoint(&need)
                } =>
                {
                    VarSet::of([s])
                }
                _ => VarSet::new(),
            };
            let nearest = match l {
                M::Own(x) => match x.view() {
                    Lit::Nearest { n, a, .. } => Some((n, a)),
                    _ => None,
                },
                _ => None,
            };
            let climb = nearest.is_some_and(|(_, a)| need.contains(a));
            need = need.minus(&gives);
            match (nearest, cmp_right) {
                (Some((n, a)), _) => {
                    need.insert(if climb { n } else { a });
                }
                (None, Some((_, rt))) => {
                    if split.is_empty() {
                        term_vars(rt, &mut need);
                    } else {
                        need.extend(&split);
                    }
                }
                _ => {}
            }
        }
        let (mut bound, mut body) = (VarSet::new(), Vec::new());
        for ((l, _), k) in pre.iter().zip(&ks) {
            if !k {
                continue;
            }
            if l.positive(v).is_some() {
                body.push(l.clone());
                bound.extend(&l.vars(r));
            } else if let Some(b) = binds(v, r, l, &bound) {
                body.push(l.clone());
                bound.extend(&b);
            }
        }
        if !key.is_subset(&bound) {
            return Err(Error::UnboundDemand {
                message: format!("demand for {} is unbound: [{}]", mh.0, pre.iter().map(|p| p.0.show(v, r)).collect::<Vec<_>>().join(", ")),
                relation: mh.0.clone(),
            });
        }
        Ok(body)
    };
    let build = |keep: &[Vec<bool>]| -> Result<Vec<Made<'v>>, Error> {
        // The first of equal magic rules stays.
        let (mut out, mut keys) = (Vec::new(), BTreeSet::new());
        for ((mh, r, pre), ks) in magic.iter().zip(keep) {
            let mut body = one(mh, *r, pre, ks)?;
            let text = format!("{}({}) {}", mh.0, mh.1.iter().map(|t| show_tm(v, *r, *t)).collect::<Vec<_>>().join(", "), body.iter().map(|l| l.show(v, *r)).collect::<Vec<_>>().join(", "));
            if keys.insert(text) {
                if body.is_empty() {
                    body.push(M::True);
                }
                out.push(Made { rule: *r, head: Some(mh.clone()), body });
            }
        }
        out.extend(rules.iter().cloned());
        out.extend(views.iter().cloned());
        Ok(out)
    };
    let settle = |keep: &[Vec<bool>]| -> Result<((Vec<BTreeSet<String>>, Edges), Vec<Made<'v>>), (Error, Vec<Made<'v>>)> {
        let made = build(keep).map_err(|e| (e, Vec::new()))?;
        match strata(v, cx.about, &made) {
            Ok(s) => Ok((s, made)),
            Err(e) => Err((e, made)),
        }
    };
    let settled = match settle(&keep) {
        Ok((settled, _)) => settled,
        Err((error, made)) => {
            let Some((scc, _)) = error.unstratified() else { return Err(Attempt::Failed(error)) };
            let negs = made
                .iter()
                .filter(|m| scc.contains(m.head.as_ref().map_or_else(|| v.rel_name(m.rule.head().rel()), |(rel, _)| rel.as_str())))
                .flat_map(|m| &m.body)
                .filter(|l| matches!(l.atom(v), Some((_, true, _)) | Some((_, _, true))))
                .filter_map(|l| l.atom(v).map(|(rel, _, _)| rel))
                .filter(|rel| scc.contains(*rel))
                .map(str::to_owned)
                .collect();
            return Err(Attempt::Cycle { error, negs });
        }
    };
    let places = |settled: &(Vec<BTreeSet<String>>, Edges)| -> (BTreeMap<String, usize>, BTreeSet<usize>) {
        let (sccs, edges) = settled;
        let comp: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.clone(), i))).collect();
        let negated = edges.iter().filter_map(|(x, ws)| comp.get(x).map(|c| (*c, ws))).filter(|(c, ws)| ws.iter().any(|(w, neg)| *neg && comp.get(w) == Some(c))).map(|(c, _)| c).collect();
        (comp, negated)
    };
    let (mut comp, mut negated) = places(&settled);
    let whole_check = std::env::var_os("RERAIL_MAGIC_CHECK").is_some();
    let edges_of = |body: &[M<'_>], comp: &BTreeMap<String, usize>| -> BTreeSet<(String, bool)> {
        body.iter().filter_map(|l| l.atom(v).filter(|(rel, _, _)| comp.contains_key(*rel)).map(|(rel, neg, agg)| (rel.to_owned(), neg || agg))).collect()
    };
    for i in 0..magic.len() {
        for j in 0..magic[i].2.len() {
            if !magic[i].2[j].1 {
                continue;
            }
            let (mh, r, pre) = &magic[i];
            let before = one(mh, *r, pre, &keep[i]);
            keep[i][j] = true;
            let after = one(mh, *r, pre, &keep[i]);
            let unchanged = comp.get(mh.0.as_str()).is_some_and(|&head| match (&before, &after) {
                (Ok(before), Ok(after)) => {
                    let was = edges_of(before, &comp);
                    edges_of(after, &comp).difference(&was).all(|(w, neg)| {
                        let at = comp[w];
                        at < head || (at == head && !*neg && !negated.contains(&head))
                    })
                }
                _ => false,
            });
            if unchanged && !whole_check {
                continue;
            }
            match settle(&keep) {
                Ok((settled, _)) => (comp, negated) = places(&settled),
                Err((e, _)) if e.unstratified().is_some() => {
                    assert!(!unchanged, "magic: a literal kept without a whole check unstratifies the program: {e}");
                    keep[i][j] = false
                }
                Err((e, _)) => return Err(Attempt::Failed(e)),
            }
        }
    }
    let made = match settle(&keep) {
        Ok((_, made)) => made,
        Err((e, _)) => return Err(Attempt::Failed(e)),
    };
    let extras_kept = magic.iter().zip(&keep).map(|((_, _, pre), ks)| pre.iter().zip(ks).filter(|((_, e), k)| *e && **k).count()).sum();
    let report = Report {
        adornments: seen.iter().filter(|(_, a)| a.contains('b')).map(|(r, a)| format!("{r}_{a}")).collect::<BTreeSet<_>>().into_iter().collect(),
        magic_rules: magic.len(),
        full: full.iter().filter(|r| cx.idb.contains(r.as_str())).cloned().collect(),
        extras_kept,
        gates: 0,
        full_for_negation: Vec::new(),
    };
    Ok((made, decls, report))
}

/// Writes rule `m` through `w`.
fn write<'v>(v: &'v Version, w: RuleBuilder<'_, '_>, m: &Made<'v>) -> std::result::Result<(), String> {
    let r = m.rule;
    // The fresh variables, `_m0` up to the last the rule names.
    let last = m.body.iter().filter_map(|l| match l {
        M::Call(n, _) => Some(*n),
        _ => None,
    });
    let names: Vec<String> = (0..=last.max().unwrap_or(0)).map(|n| format!("_m{n}")).collect();
    let name_of = |n: u32| -> &str { names.get(n as usize).map_or("_m", String::as_str) };
    let tm = |t: Tm<'v>| -> T<'_> {
        match t {
            Tm::Own(t) => t_of(v, r, t),
            Tm::Fresh(n) => T::Var(name_of(n)),
        }
    };
    let mut w = match &m.head {
        Some((rel, args)) => {
            let a: Vec<T<'_>> = args.iter().map(|t| tm(*t)).collect();
            w.head(rel.as_str(), &a)
        }
        None => w.head_from(v, r, r.head()),
    };
    for l in &m.body {
        w = match l {
            M::Own(x) => w.lit(v, r, *x),
            M::Guard(rel, args) => {
                let a: Vec<T<'_>> = args.iter().map(|t| tm(*t)).collect();
                w.atom(rel.as_str(), &a)
            }
            M::Call(n, call) => w.cmp(T::Var(name_of(*n)), CmpOp::Eq, t_of(v, r, *call)),
            M::True => w.atom("magic_true", &[T::Int(0)]),
        };
    }
    w.finish()?;
    Ok(())
}
