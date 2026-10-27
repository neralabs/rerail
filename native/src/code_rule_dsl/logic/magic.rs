//! Demand: magic sets (03 §12 "Demand / magic sets"; 05 Phase 6 row 4).
//!
//! A port of Typing-support's `rdl.py` (T) `magic_sets`, `magic_sets_once`, `magic_adorn`, `magic_adorn_once` and
//! `gate_negations`, rule for rule: its transformed programs on T's five programs are the tests' goldens. T's
//! environment switches are `Options`, with T's defaults. Outstanding (C10): demand is still whole-program; the
//! waves (Phase 7) install each wave's magic relations as its demand batch.
//!
//! - `.demand r, …` (`magic_sets`): each demanded relation restricted to the keys (first column) its consumers ask
//!   for, with magic rules passing as many literals sideways as stay stratifiable.
//! - `.demand *` (`magic_adorn`): Soufflé's transformation on every derived relation, with T's differences (one
//!   relation per predicate, key adornments, sideways order, computed keys, negation kept), documented on T's
//!   `magic_adorn` and below.
//! - Gated negations (`Options::gate`): a negation that demand puts inside its own component waits for its relation's
//!   `done_` gate, which the native engine opens once what the relation reads is quiet.

use super::front::{Atom, Error, Literal, Program, Rule, Term, Value, cmp_binds, join, strata, tarjan};
use std::collections::{BTreeMap, BTreeSet};

/// T's environment switches.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Options {
    /// `RDL_COARSE`: magic rules take no extras.
    pub(in crate::code_rule_dsl) coarse: bool,
    /// `RDL_MAGIC_KEY`: adorn the key column only.
    pub(in crate::code_rule_dsl) key_only: bool,
    /// `RDL_MAGIC_NONKEY`: adorn non-key columns bound by a variable when the rules answer them.
    pub(in crate::code_rule_dsl) nonkey: bool,
    /// `RDL_MAGIC_INPUT_FIRST`: sideways, an input on a bound key before a derived atom.
    pub(in crate::code_rule_dsl) input_first: bool,
    /// `RDL_GATE` (with the native program mode): gate negations instead of computing relations in full.
    pub(in crate::code_rule_dsl) gate: bool,
    /// `RDL_MAGIC_EXCLUDE`: relations computed in full.
    pub(in crate::code_rule_dsl) exclude: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self { coarse: false, key_only: false, nonkey: true, input_first: true, gate: false, exclude: Vec::new() }
    }
}

/// What `magic_adorn` did (T's `magic_report`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::code_rule_dsl) struct Report {
    pub(in crate::code_rule_dsl) adornments: Vec<String>,
    pub(in crate::code_rule_dsl) magic_rules: usize,
    pub(in crate::code_rule_dsl) full: Vec<String>,
    pub(in crate::code_rule_dsl) extras_kept: usize,
    pub(in crate::code_rule_dsl) gates: usize,
    pub(in crate::code_rule_dsl) full_for_negation: Vec<String>,
}

/// A gated negation's gate: `done` copies `src` (the negated relation's demand) once `quiet` is quiet, after the
/// gates in `deps`; `src` is `None` for a unit gate (`done_N(0)`).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Gate {
    pub(in crate::code_rule_dsl) src: Option<String>,
    pub(in crate::code_rule_dsl) rel: String,
    pub(in crate::code_rule_dsl) level: usize,
    pub(in crate::code_rule_dsl) quiet: Vec<String>,
    pub(in crate::code_rule_dsl) deps: Vec<String>,
}

fn vars(t: &Term) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    t.vars(&mut out);
    out
}

fn vars_all(ts: &[Term]) -> BTreeSet<String> {
    ts.iter().flat_map(vars).collect()
}

fn is_const(t: &Term) -> bool {
    matches!(t, Term::Int(_) | Term::Str(_))
}

fn atom(rel: String, args: Vec<Term>) -> Atom {
    Atom { rel, args, neg: false, gated: false }
}

fn positive(l: &Literal) -> Option<&Atom> {
    match l {
        Literal::Atom(a) if !a.neg => Some(a),
        _ => None,
    }
}

fn magic_true() -> Literal {
    Literal::Atom(atom("magic_true".to_owned(), vec![Term::Int(0)]))
}

fn add_magic_true(prog: &mut Program) {
    prog.decls.insert("magic_true".to_owned(), vec![("k".to_owned(), "int".to_owned())]);
    prog.facts.entry("magic_true".to_owned()).or_default().push(vec![Value::Int(0)]);
}

/// The variables `l` binds once it runs, or `None` if it cannot run yet (the planner's `ready`).
fn binds(l: &Literal, bound: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    match l {
        Literal::Atom(a) if !a.neg => Some(a.vars()),
        Literal::Atom(a) => a.vars().is_subset(bound).then(BTreeSet::new),
        Literal::Cmp { l, op, r } => cmp_binds(l, op, r, bound),
        Literal::Agg { var, atom, .. } => {
            let mut need = atom.vars();
            need.remove(var);
            need.is_subset(bound).then(|| BTreeSet::from([var.clone()]))
        }
        Literal::Nearest { n, a, .. } => (bound.contains(n) || bound.contains(a)).then(|| l.vars()),
        Literal::Ast { .. } => Some(l.vars()),
    }
}

/// `.demand r, …`: applies demand, dropping a demanded relation whose negated use would close a negative cycle
/// through its own demand; that relation is computed in full (`demand_dropped` says why).
pub(in crate::code_rule_dsl) fn magic_sets(prog: Program, options: &Options) -> Result<Program, Error> {
    let mut dropped: Vec<(String, String)> = Vec::new();
    loop {
        let mut p = prog.clone();
        p.demand.retain(|r| !dropped.iter().any(|(d, _)| d == r));
        let demand = p.demand.clone();
        match magic_sets_once(p, options) {
            Ok(mut p) => {
                p.demand_dropped = dropped;
                return Ok(p);
            }
            Err(e) => {
                if let Some((_, negated)) = e.unstratified() {
                    if !demand.iter().any(|d| d == negated) {
                        return Err(e);
                    }
                    dropped.push((negated.to_owned(), "negative cycle through its demand".to_owned()));
                } else if let Error::UnboundDemand { relation, .. } = &e {
                    // Only a consequence of an earlier drop; otherwise the program asks for unbounded demand.
                    let Some((last, _)) = dropped.last() else { return Err(e) };
                    let why = format!("a use with an unbound key once {last} is computed in full");
                    dropped.push((relation.clone(), why));
                } else {
                    return Err(e);
                }
            }
        }
    }
}

/// Restricts each `.demand` relation to the keys its consumers ask for (first column bound). Every rule for a
/// demanded R gets `magic_R(key)` first; every use of a demanded S with its key bound produces `magic_S(key) :- <the
/// literals before it>`, a negated use too. Extras (negations, and atoms of the head's own recursive component) are
/// added greedily while the program stays stratifiable; leaving one out only widens demand.
fn magic_sets_once(mut prog: Program, options: &Options) -> Result<Program, Error> {
    if prog.demand.is_empty() {
        return Ok(prog);
    }
    let (sccs, _) = strata(&prog)?;
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.as_str(), i))).collect();
    let demanded = |r: &str| prog.demand.iter().any(|d| d == r);
    let (mut rules, mut magic) = (Vec::new(), Vec::<(Atom, Vec<(Literal, bool)>)>::new());
    for rule in &prog.rules {
        let h = &rule.head;
        let demanded_head = demanded(&h.rel);
        let (mut seen, mut bound) = (Vec::new(), BTreeSet::new());
        if demanded_head {
            let key = h.args[0].clone();
            bound.extend(vars(&key));
            seen.push((Literal::Atom(atom(format!("magic_{}", h.rel), vec![key])), false));
        }
        for l in &rule.body {
            if let Literal::Atom(a) = l
                && demanded(&a.rel)
            {
                let key = &a.args[0];
                if !(vars(key).is_subset(&bound) || is_const(key)) {
                    return Err(Error::UnboundDemand {
                        message: format!("demand for {} is unbound in the rule for {}: [{}]", a.rel, h.rel, join(&rule.body)),
                        relation: a.rel.clone(),
                    });
                }
                magic.push((atom(format!("magic_{}", a.rel), vec![key.clone()]), seen.clone()));
            }
            match l {
                Literal::Atom(a) if !a.neg => {
                    let same = demanded(&a.rel) && comp.get(a.rel.as_str()) == comp.get(h.rel.as_str());
                    seen.push((l.clone(), same));
                    bound.extend(a.vars());
                }
                Literal::Atom(a) if a.vars().is_subset(&bound) => seen.push((l.clone(), true)),
                Literal::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a) => {
                    seen.push((l.clone(), false));
                    bound.extend(l.vars());
                }
                Literal::Cmp { .. } if l.vars().is_subset(&bound) => seen.push((l.clone(), false)),
                _ => {}
            }
        }
        let mut body = Vec::new();
        if demanded_head {
            body.push(Literal::Atom(atom(format!("magic_{}", h.rel), vec![h.args[0].clone()])));
        }
        body.extend(rule.body.iter().cloned());
        rules.push(Rule { head: h.clone(), body });
    }
    for r in prog.demand.clone() {
        let ty = prog.decls[&r][0].1.clone();
        prog.decls.insert(format!("magic_{r}"), vec![("k".to_owned(), ty)]);
    }
    add_magic_true(&mut prog);
    let mut keep: Vec<Vec<bool>> = magic.iter().map(|(_, pre)| pre.iter().map(|(_, extra)| !extra).collect()).collect();
    let build = |keep: &[Vec<bool>]| -> Result<Vec<Rule>, Error> {
        let mut out = Vec::new();
        for ((mh, pre), ks) in magic.iter().zip(keep) {
            // A kept literal still needs its variables bound by the kept literals before it.
            let (mut bound, mut body) = (BTreeSet::new(), Vec::new());
            for ((l, _), k) in pre.iter().zip(ks) {
                if !k {
                    continue;
                }
                match l {
                    Literal::Atom(a) if !a.neg => {
                        body.push(l.clone());
                        bound.extend(a.vars());
                    }
                    Literal::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a) => {
                        body.push(l.clone());
                        bound.extend(l.vars());
                    }
                    _ if l.vars().is_subset(&bound) => body.push(l.clone()),
                    _ => {}
                }
            }
            if !vars(&mh.args[0]).is_subset(&bound) && !is_const(&mh.args[0]) {
                return Err(Error::UnboundDemand {
                    message: format!("demand for {} is unbound: [{}]", &mh.rel[6..], join(&pre.iter().map(|p| p.0.clone()).collect::<Vec<_>>())),
                    relation: mh.rel[6..].to_owned(),
                });
            }
            if body.is_empty() {
                body.push(magic_true());
            }
            out.push(Rule { head: mh.clone(), body });
        }
        out.extend(rules.iter().cloned());
        Ok(out)
    };
    // Greedy: start from no extras (always stratifiable) and keep each extra that leaves the program stratifiable.
    prog.rules = build(&keep)?;
    strata(&prog)?;
    for i in 0..magic.len() {
        for j in 0..magic[i].1.len() {
            if !magic[i].1[j].1 || options.coarse {
                continue;
            }
            keep[i][j] = true;
            prog.rules = build(&keep)?;
            match strata(&prog) {
                Ok(_) => {}
                Err(e) if e.unstratified().is_some() => keep[i][j] = false,
                Err(e) => return Err(e),
            }
        }
    }
    prog.rules = build(&keep)?;
    prog.magic_extras = Some(
        magic.iter().zip(&keep).map(|((_, pre), ks)| pre.iter().zip(ks).filter(|((_, extra), k)| *extra && **k).count()).sum(),
    );
    Ok(prog)
}

/// A failure of one `magic_adorn_once` attempt: a negative cycle that demand closed (with the relations negated or
/// aggregated inside it), or any other error.
enum Attempt {
    Cycle { error: Error, negs: BTreeSet<String> },
    Failed(Error),
}

impl From<Error> for Attempt {
    fn from(e: Error) -> Self {
        Attempt::Failed(e)
    }
}

/// `.demand *`: Soufflé's magic-set transformation on every derived relation (T's `magic_adorn`; see the module
/// docs). A negative cycle that demand closes makes each relation negated inside it, with what it depends on,
/// computed in full, and the transformation is tried again.
pub(in crate::code_rule_dsl) fn magic_adorn(prog: Program, options: &Options) -> Result<Program, Error> {
    let derived: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).collect();
    let mut by: BTreeMap<String, Vec<Rule>> = BTreeMap::new();
    for r in &prog.rules {
        by.entry(r.head.rel.clone()).or_default().push(r.clone());
    }
    let view: BTreeSet<String> = by
        .iter()
        .filter(|(_, rs)| {
            rs.iter().all(|r| r.body.len() == 1 && positive(&r.body[0]).is_some_and(|a| !derived.contains(&a.rel)))
        })
        .map(|(r, _)| r.clone())
        .collect();
    let idb: BTreeSet<String> = derived.difference(&view).cloned().collect();
    let mut full: BTreeSet<String> = options.exclude.iter().cloned().collect();
    // A relation that aggregates is computed in full: guarded, its count would join its demand's recursive component.
    full.extend(prog.rules.iter().filter(|r| r.body.iter().any(|l| matches!(l, Literal::Agg { .. }))).map(|r| r.head.rel.clone()));
    let (sccs, _) = strata(&prog)?;
    let level: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.clone(), i))).collect();
    let mut cyc = Vec::new();
    loop {
        let attempt = Adorn { prog: prog.clone(), by: &by, idb: &idb, level: &level, options, answered: BTreeMap::new() }.run(&full);
        match attempt {
            Ok(mut p) => {
                if let Some(report) = &mut p.magic_report {
                    report.full_for_negation = cyc;
                }
                return Ok(p);
            }
            Err(Attempt::Failed(e)) => return Err(e),
            Err(Attempt::Cycle { error, negs }) => {
                // Each relation negated inside the component is computed in full, with every relation it depends on,
                // so that its demand no longer comes from its consumers.
                let mut new = BTreeSet::new();
                let mut todo: Vec<String> = negs.iter().filter(|r| idb.contains(*r) && !full.contains(*r)).cloned().collect();
                while let Some(r) = todo.pop() {
                    if new.contains(&r) || full.contains(&r) || !idb.contains(&r) {
                        continue;
                    }
                    todo.extend(by[&r].iter().flat_map(|rule| rule.body.iter().filter_map(Literal::atom).map(|a| a.rel.clone())));
                    new.insert(r);
                }
                if new.is_empty() {
                    return Err(error);
                }
                cyc.extend(negs.intersection(&new).cloned());
                full.extend(new);
            }
        }
    }
}

struct Adorn<'a> {
    prog: Program,
    by: &'a BTreeMap<String, Vec<Rule>>,
    idb: &'a BTreeSet<String>,
    level: &'a BTreeMap<String, usize>,
    options: &'a Options,
    /// `answers`' results for the adornments asked at the top level, per `closure`.
    answered: BTreeMap<(String, String), bool>,
}

type Magic = Vec<(Atom, Vec<(Literal, bool)>)>;

fn bset(ad: &str) -> BTreeSet<usize> {
    ad.bytes().enumerate().filter(|(_, x)| *x == b'b').map(|(i, _)| i).collect()
}

impl Adorn<'_> {
    fn arity(&self, rel: &str) -> usize {
        self.prog.decls[rel].len()
    }

    /// A relation's key columns: all but a lattice's value.
    fn keyed(&self, rel: &str) -> usize {
        self.prog.lattice.get(rel).map_or_else(|| self.arity(rel), |l| l.keys)
    }

    fn pattern(&self, a: &Atom, bound: &BTreeSet<String>) -> String {
        let keyed = self.keyed(&a.rel);
        a.args
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let b = i < keyed && (is_const(t) || matches!(t, Term::Var(v) if bound.contains(v)));
                if b { 'b' } else { 'f' }
            })
            .collect()
    }

    fn var_bound(a: &Atom, ad: &str) -> bool {
        a.args.iter().zip(ad.bytes()).any(|(t, x)| x == b'b' && matches!(t, Term::Var(_)))
    }

    /// `b` for each bound argument; all free unless the key is bound (demand on later columns alone restricts
    /// little and reverses the rules' direction), or a non-key column is bound by a variable and the rules answer
    /// that adornment.
    fn adorn(&mut self, a: &Atom, bound: &BTreeSet<String>, full: &BTreeSet<String>) -> String {
        let ad = self.pattern(a, bound);
        if !ad.starts_with('b') {
            let free = "f".repeat(ad.len());
            return if self.options.nonkey && Self::var_bound(a, &ad) && self.answers(&a.rel, &ad, &[], full) { ad } else { free };
        }
        if self.options.key_only { format!("b{}", "f".repeat(ad.len() - 1)) } else { ad }
    }

    /// Whether R's rules answer the non-key adornment `ad` without asking any derived relation in full, or through a
    /// cycle of non-key adornments.
    fn answers(&mut self, rel: &str, ad: &str, stack: &[(String, String)], full: &BTreeSet<String>) -> bool {
        let key = (rel.to_owned(), ad.to_owned());
        if let Some(&ok) = self.answered.get(&key) {
            return ok;
        }
        if full.contains(rel) || stack.contains(&key) {
            return false;
        }
        let mut ok = true;
        'rules: for rule in self.by.get(rel).into_iter().flatten() {
            let (mut bound, mut extra) = (BTreeSet::new(), Vec::new());
            for (t, x) in rule.head.args.iter().zip(ad.bytes()) {
                if x != b'b' {
                    continue;
                }
                let mut t = t.clone();
                if let Term::Call(..) = t {
                    let v = Term::Var(format!("_m{}", extra.len()));
                    extra.push(Literal::Cmp { l: v.clone(), op: "=", r: t });
                    t = v;
                }
                if let Term::Var(v) = t {
                    bound.insert(v);
                }
            }
            let mut body = rule.body.clone();
            body.extend(extra);
            let Some(order) = self.sips(&body, &bound) else {
                ok = false;
                break;
            };
            for l in &order {
                if let Literal::Atom(a) = l
                    && self.idb.contains(&a.rel)
                    && !full.contains(&a.rel)
                {
                    let ad2 = self.pattern(a, &bound);
                    if !ad2.starts_with('b') {
                        let mut deeper = stack.to_vec();
                        deeper.push(key.clone());
                        if !(Self::var_bound(a, &ad2) && self.answers(&a.rel, &ad2, &deeper, full)) {
                            ok = false;
                            break 'rules;
                        }
                    }
                }
                bound.extend(binds(l, &bound).unwrap_or_default());
            }
        }
        if stack.is_empty() {
            self.answered.insert(key, ok);
        }
        ok
    }

    /// The body in sideways order: filters as soon as they can run; then the left-most positive atom whose key is
    /// bound, an input before a derived one; else one sharing a bound variable (an input or view first); else the
    /// lowest one in the strata (inputs first). `None` if no order binds the rest.
    fn sips(&self, body: &[Literal], bound: &BTreeSet<String>) -> Option<Vec<Literal>> {
        let (mut rest, mut order, mut bound) = (body.to_vec(), Vec::new(), bound.clone());
        while !rest.is_empty() {
            if let Some(i) = rest.iter().position(|l| positive(l).is_none() && binds(l, &bound).is_some()) {
                let l = rest.remove(i);
                bound.extend(binds(&l, &bound).unwrap_or_default());
                order.push(l);
                continue;
            }
            let pos: Vec<usize> = (0..rest.len()).filter(|&i| positive(&rest[i]).is_some()).collect();
            if pos.is_empty() {
                return None;
            }
            let a = |i: usize| positive(&rest[i]).expect("a positive atom");
            let first = |i: usize| is_const(&a(i).args[0]) || matches!(&a(i).args[0], Term::Var(v) if bound.contains(v));
            let derived = |i: usize| self.idb.contains(&a(i).rel);
            let joined: Vec<usize> = pos.iter().copied().filter(|&i| !a(i).vars().is_disjoint(&bound)).collect();
            let lowest = |ix: &[usize], key: &dyn Fn(usize) -> i64| {
                ix.iter().copied().fold(None, |best: Option<usize>, i| match best {
                    Some(b) if key(b) <= key(i) => Some(b),
                    _ => Some(i),
                })
            };
            let pick = pos
                .iter()
                .copied()
                .find(|&i| first(i) && !derived(i) && self.options.input_first)
                .or_else(|| pos.iter().copied().find(|&i| first(i)))
                .or_else(|| lowest(&joined, &|i| i64::from(derived(i))))
                .or_else(|| lowest(&pos, &|i| self.level.get(&a(i).rel).map_or(-1, |&l| l as i64)))
                .expect("a positive atom");
            let l = rest.remove(pick);
            bound.extend(l.vars());
            order.push(l);
        }
        Some(order)
    }

    /// The adorned rules and magic rules reachable from the outputs (all free), and the adornments seen.
    fn closure(
        &mut self,
        full: &BTreeSet<String>,
        coarse: &BTreeMap<(String, String), String>,
    ) -> Result<(Vec<Rule>, Magic, BTreeSet<(String, String)>), Error> {
        let (mut rules, mut magic, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
        self.answered.clear();
        let mut todo: Vec<(String, String)> =
            self.prog.outputs.iter().filter(|o| self.idb.contains(*o)).map(|o| (o.clone(), "f".repeat(self.arity(o)))).collect();
        while let Some((rel, mut ad)) = todo.pop() {
            if full.contains(&rel) {
                ad = "f".repeat(self.arity(&rel));
            }
            if !seen.insert((rel.clone(), ad.clone())) {
                continue;
            }
            for rule in self.by[&rel].clone() {
                let (mut bound, mut pre, mut guard) = (BTreeSet::new(), Vec::<(Literal, bool)>::new(), None);
                let mut body = rule.body.clone();
                if ad.contains('b') {
                    // A computed head term at a bound position is matched through a fresh variable: the guard binds
                    // it, and `$m = @cat($q, $x)` tests it once the body binds the call's arguments.
                    let (mut gargs, mut calls) = (Vec::new(), Vec::new());
                    for (t, x) in rule.head.args.iter().zip(ad.bytes()) {
                        if x != b'b' {
                            continue;
                        }
                        let mut t = t.clone();
                        if let Term::Call(..) = t {
                            let v = Term::Var(format!("_m{}", calls.len()));
                            calls.push(Literal::Cmp { l: v.clone(), op: "=", r: t });
                            t = v;
                        }
                        gargs.push(t);
                    }
                    let g = atom(format!("magic_{rel}_{ad}"), gargs);
                    body.extend(calls);
                    bound.extend(g.vars());
                    pre.push((Literal::Atom(g.clone()), false));
                    guard = Some(g);
                }
                let order = self.sips(&body, &bound).ok_or_else(|| Error::Syntax(format!("no order binds the body of {rule}")))?;
                for l in order {
                    let a = l.atom().cloned();
                    if let Some(a) = &a
                        && self.idb.contains(&a.rel)
                    {
                        // An aggregated relation is computed in full: its demand would pull the count into its
                        // consumers' recursive component.
                        let ad2 = if full.contains(&a.rel) || matches!(l, Literal::Agg { .. }) {
                            "f".repeat(self.arity(&a.rel))
                        } else {
                            self.adorn(a, &bound, full)
                        };
                        let ad2 = coarse.get(&(a.rel.clone(), ad2.clone())).cloned().unwrap_or(ad2);
                        if ad2.contains('b') {
                            let args = a.args.iter().zip(ad2.bytes()).filter(|(_, x)| *x == b'b').map(|(t, _)| t.clone()).collect();
                            magic.push((atom(format!("magic_{}_{ad2}", a.rel), args), pre.clone()));
                        }
                        todo.push((a.rel.clone(), ad2));
                    }
                    // Sideways: input atoms and filters always; derived atoms and negations as extras, kept if the
                    // program stays stratifiable. Leaving one out only widens the demand.
                    match &l {
                        Literal::Atom(x) if !x.neg => pre.push((l.clone(), self.idb.contains(&x.rel))),
                        Literal::Agg { .. } => {}
                        _ => pre.push((l.clone(), matches!(l, Literal::Atom(_)))),
                    }
                    let b2 = binds(&l, &bound).unwrap_or_default();
                    bound.extend(b2);
                }
                let mut out = Vec::new();
                out.extend(guard.map(Literal::Atom));
                out.extend(body);
                rules.push(Rule { head: rule.head.clone(), body: out });
            }
        }
        Ok((rules, magic, seen))
    }

    fn run(mut self, full_in: &BTreeSet<String>) -> Result<Program, Attempt> {
        // A relation demanded with every argument free anywhere is computed in full; an adornment binding a superset
        // of another's columns is answered through the other (`pv_bbb` through `pv_bbf`).
        let (mut full, mut coarse) = (full_in.clone(), BTreeMap::<(String, String), String>::new());
        let (rules, magic, seen) = loop {
            let (rules, magic, seen) = self.closure(&full, &coarse)?;
            let free: BTreeSet<String> =
                seen.iter().filter(|(_, a)| !a.contains('b')).map(|(r, _)| r.clone()).filter(|r| !full.contains(r)).collect();
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
        for (rel, ad) in &seen {
            if ad.contains('b') {
                let cols = self.prog.decls[rel].iter().zip(ad.bytes()).filter(|(_, x)| *x == b'b').map(|(c, _)| c.clone()).collect();
                self.prog.decls.insert(format!("magic_{rel}_{ad}"), cols);
            }
        }
        add_magic_true(&mut self.prog);
        let mut keep: Vec<Vec<bool>> = magic.iter().map(|(_, pre)| pre.iter().map(|(_, extra)| !extra).collect()).collect();
        let views: Vec<Rule> = self.prog.rules.iter().filter(|r| !self.idb.contains(&r.head.rel)).cloned().collect();
        let build = |keep: &[Vec<bool>]| -> Result<Vec<Rule>, Error> {
            // Keyed by T's repr of (head, body): the first of equal magic rules stays.
            let (mut out, mut keys) = (Vec::new(), BTreeSet::new());
            for ((mh, pre), ks) in magic.iter().zip(keep) {
                // A left-out literal that binds a key variable, or what such a literal needs, stays.
                let key = vars_all(&mh.args);
                let (mut ks, mut need) = (ks.clone(), key.clone());
                for i in (0..pre.len()).rev() {
                    let l = &pre[i].0;
                    let gives = match l {
                        Literal::Atom(a) if !a.neg => l.vars(),
                        Literal::Nearest { .. } => l.vars(),
                        Literal::Cmp { l: Term::Var(_), op: "=", r } if r.is_cat() => l.vars(),
                        Literal::Cmp { l: Term::Var(v), op: "=", .. } => BTreeSet::from([v.clone()]),
                        _ => BTreeSet::new(),
                    };
                    if gives.is_disjoint(&need) {
                        continue;
                    }
                    ks[i] = true;
                    // A split gives its parts from the whole, and the whole from its parts.
                    let split = match l {
                        Literal::Cmp { l: Term::Var(v), r, .. } if r.is_cat() && !vars(r).is_disjoint(&need) => BTreeSet::from([v.clone()]),
                        _ => BTreeSet::new(),
                    };
                    let climb = matches!(l, Literal::Nearest { a, .. } if need.contains(a));
                    need.retain(|v| !gives.contains(v));
                    match l {
                        Literal::Nearest { n, a, .. } => {
                            need.insert(if climb { n.clone() } else { a.clone() });
                        }
                        Literal::Cmp { r, .. } => need.extend(if split.is_empty() { vars(r) } else { split }),
                        _ => {}
                    }
                }
                let (mut bound, mut body) = (BTreeSet::new(), Vec::new());
                for ((l, _), k) in pre.iter().zip(&ks) {
                    if !k {
                        continue;
                    }
                    if let Some(a) = positive(l) {
                        body.push(l.clone());
                        bound.extend(a.vars());
                    } else if let Some(b) = binds(l, &bound) {
                        body.push(l.clone());
                        bound.extend(b);
                    }
                }
                if !key.is_subset(&bound) {
                    let pre: Vec<Literal> = pre.iter().map(|p| p.0.clone()).collect();
                    return Err(Error::UnboundDemand {
                        message: format!("demand for {} is unbound: [{}]", mh.rel, join(&pre)),
                        relation: mh.rel.clone(),
                    });
                }
                if keys.insert(format!("{mh} {}", join(&body))) {
                    if body.is_empty() {
                        body.push(magic_true());
                    }
                    out.push(Rule { head: mh.clone(), body });
                }
            }
            out.extend(rules.iter().cloned());
            out.extend(views.iter().cloned());
            Ok(out)
        };
        let settle = |this: &mut Self, keep: &[Vec<bool>]| -> Result<(), Error> {
            this.prog.rules = build(keep)?;
            this.prog.gates.clear();
            match strata(&this.prog) {
                Err(e) if e.unstratified().is_some() => {
                    if !this.options.gate {
                        return Err(e);
                    }
                    this.gate_negations(&full, &coarse)?;
                    strata(&this.prog).map(drop)
                }
                other => other.map(drop),
            }
        };
        if let Err(error) = settle(&mut self, &keep) {
            let Some((scc, _)) = error.unstratified() else { return Err(Attempt::Failed(error)) };
            let negs = self
                .prog
                .rules
                .iter()
                .filter(|r| scc.contains(&r.head.rel))
                .flat_map(|r| &r.body)
                .filter(|l| matches!(l, Literal::Atom(a) if a.neg) || matches!(l, Literal::Agg { .. }))
                .filter_map(Literal::atom)
                .filter(|a| scc.contains(&a.rel))
                .map(|a| a.rel.clone())
                .collect();
            return Err(Attempt::Cycle { error, negs });
        }
        for i in 0..magic.len() {
            for j in 0..magic[i].1.len() {
                if !magic[i].1[j].1 {
                    continue;
                }
                keep[i][j] = true;
                match settle(&mut self, &keep) {
                    Ok(()) => {}
                    Err(e) if e.unstratified().is_some() => keep[i][j] = false,
                    Err(e) => return Err(Attempt::Failed(e)),
                }
            }
        }
        settle(&mut self, &keep)?;
        self.prog.demand.clear();
        let extras_kept = magic.iter().zip(&keep).map(|((_, pre), ks)| pre.iter().zip(ks).filter(|((_, e), k)| *e && **k).count()).sum();
        self.prog.magic_report = Some(Report {
            adornments: seen.iter().filter(|(_, a)| a.contains('b')).map(|(r, a)| format!("{r}_{a}")).collect::<BTreeSet<_>>().into_iter().collect(),
            magic_rules: magic.len(),
            full: full.intersection(self.idb).cloned().collect(),
            extras_kept,
            gates: self.prog.gates.len(),
            full_for_negation: Vec::new(),
        });
        Ok(self.prog)
    }

    /// The adornment a negated atom (every variable bound) was demanded with.
    fn neg_ad(&mut self, a: &Atom, full: &BTreeSet<String>, coarse: &BTreeMap<(String, String), String>) -> String {
        let ad = if full.contains(&a.rel) { "f".repeat(self.arity(&a.rel)) } else { self.adorn(a, &a.vars(), full) };
        coarse.get(&(a.rel.clone(), ad.clone())).cloned().unwrap_or(ad)
    }

    /// Gates the negations that demand puts in their own component (T's `gate_negations`): `!N(t)` becomes
    /// `done_N(t), !N(t)`, and the native engine copies `magic_N` into `done_N` only in a wave after which nothing N
    /// reads changed. Within each such component the edges into guards are dropped: what is left must be
    /// stratified, and its strata are the gates' levels.
    fn gate_negations(&mut self, full: &BTreeSet<String>, coarse: &BTreeMap<(String, String), String>) -> Result<(), Error> {
        let derived: BTreeSet<String> = self.prog.rules.iter().map(|r| r.head.rel.clone()).collect();
        let mut edges: BTreeMap<String, BTreeSet<(String, bool)>> = BTreeMap::new();
        for r in &self.prog.rules {
            for l in &r.body {
                if let Some(a) = l.atom()
                    && derived.contains(&a.rel)
                {
                    edges.entry(r.head.rel.clone()).or_default().insert((a.rel.clone(), a.neg || matches!(l, Literal::Agg { .. })));
                }
            }
        }
        let sccs = tarjan(&derived, &edges);
        let comp: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.clone(), i))).collect();
        let bad: BTreeSet<usize> = derived
            .iter()
            .flat_map(|v| edges.get(v).into_iter().flatten().filter(|(w, neg)| *neg && comp[w] == comp[v]).map(|_| comp[v]))
            .collect();
        let guard = |v: &str, w: &str| w.starts_with("magic_") && !v.starts_with("magic_");
        let (mut red, mut level) = (BTreeMap::<String, BTreeSet<(String, bool)>>::new(), BTreeMap::<String, usize>::new());
        for &i in &bad {
            let scc = &sccs[i];
            let e: BTreeMap<String, BTreeSet<(String, bool)>> = scc
                .iter()
                .map(|v| {
                    let inner = edges.get(v).into_iter().flatten().filter(|(w, _)| scc.contains(w) && !guard(v, w)).cloned().collect();
                    (v.clone(), inner)
                })
                .collect();
            for (k, s) in tarjan(scc, &e).iter().enumerate() {
                for v in s {
                    level.insert(v.clone(), k);
                }
            }
            for v in scc {
                for (w, n) in &e[v] {
                    if *n && level[w] >= level[v] {
                        return Err(Error::NotStratifiable {
                            message: format!("not stratifiable: {v} depends negatively on {w} without its demand"),
                            component: scc.clone(),
                            negated: w.clone(),
                        });
                    }
                }
            }
            red.extend(e);
        }
        let (mut rules, mut gates) = (Vec::new(), Vec::<(String, Gate)>::new());
        for r in self.prog.rules.clone() {
            let mut nb = Vec::new();
            for l in r.body {
                let inside = l.atom().is_some_and(|a| {
                    comp.get(&a.rel).is_some_and(|c| Some(c) == comp.get(&r.head.rel) && bad.contains(c))
                });
                let agg = matches!(l, Literal::Agg { .. });
                if !(inside && (agg || l.atom().is_some_and(|a| a.neg))) {
                    nb.push(l);
                    continue;
                }
                let a = l.atom().expect("an atom").clone();
                // An aggregated relation is full.
                let ad = if agg { "f".repeat(a.args.len()) } else { self.neg_ad(&a, full, coarse) };
                let mut src = Some(format!("magic_{}_{ad}", a.rel));
                let g = if ad.contains('b') && comp.get(src.as_deref().unwrap_or_default()) == comp.get(&a.rel) {
                    atom(format!("done_{}_{ad}", a.rel), a.args.iter().zip(ad.bytes()).filter(|(_, x)| *x == b'b').map(|(t, _)| t.clone()).collect())
                } else {
                    src = None;
                    atom(format!("done_{}", a.rel), vec![Term::Int(0)])
                };
                if !gates.iter().any(|(name, _)| *name == g.rel) {
                    gates.push((g.rel.clone(), Gate { src, rel: a.rel.clone(), level: level[&a.rel], quiet: Vec::new(), deps: Vec::new() }));
                }
                let n = match l {
                    Literal::Agg { var, op, over, atom } => Literal::Agg { var, op, over, atom: Atom { gated: true, ..atom } },
                    _ => Literal::Atom(Atom { neg: true, gated: true, ..a }),
                };
                nb.push(Literal::Atom(g));
                nb.push(n);
            }
            rules.push(Rule { head: r.head, body: nb });
        }
        let names: BTreeSet<String> = gates.iter().map(|(n, _)| n.clone()).collect();
        for i in 0..gates.len() {
            let (done, g) = gates[i].clone();
            if let Some(src) = &g.src {
                let cols = self.prog.decls[src].clone();
                let vs: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("_g{i}"))).collect();
                self.prog.decls.insert(done.clone(), cols);
                rules.push(Rule { head: atom(done.clone(), vs.clone()), body: vec![Literal::Atom(atom(src.clone(), vs))] });
            } else {
                self.prog.decls.insert(done.clone(), vec![("k".to_owned(), "int".to_owned())]);
                let wild = vec![Term::Wild; self.prog.decls[&g.rel].len()];
                rules.push(Rule { head: atom(done.clone(), vec![Term::Int(0)]), body: vec![Literal::Atom(atom(g.rel.clone(), wild))] });
            }
            let (mut reach, mut todo) = (BTreeSet::new(), vec![g.rel.clone()]);
            while let Some(v) = todo.pop() {
                if reach.insert(v.clone()) {
                    todo.extend(red.get(&v).into_iter().flatten().map(|(w, _)| w.clone()));
                }
            }
            let scc = &sccs[comp[&g.rel]];
            let magic_of = |m: &str| m.strip_prefix("magic_").and_then(|m| m.rsplit_once('_')).map(|(r, _)| r.to_owned());
            let mut quiet: BTreeSet<String> = reach.clone();
            quiet.extend(scc.iter().filter(|m| magic_of(m).is_some_and(|r| reach.contains(&r))).cloned());
            let deps: BTreeSet<String> = rules
                .iter()
                .filter(|r| reach.contains(&r.head.rel))
                .flat_map(|r| &r.body)
                .filter_map(|l| if let Literal::Atom(a) = l { Some(&a.rel) } else { None })
                .filter(|rel| names.contains(*rel) && **rel != done)
                .cloned()
                .collect();
            gates[i].1.quiet = quiet.into_iter().collect();
            gates[i].1.deps = deps.into_iter().collect();
        }
        self.prog.rules = rules;
        self.prog.gates = gates;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
