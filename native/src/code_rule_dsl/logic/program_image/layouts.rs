//! Each relation's layout (decision 0361 step 3): the facts its rows hold, declared by `.layout` or inferred from its
//! rules, each with where it came from. Inference is the declared facts carried through a rule, as types are:
//!
//! - **a key** (a functional dependency) holds of a head where one body atom's key determines, through the body's
//!   keys and equalities, every variable the head holds: a projection that keeps a key, a join along a key, a
//!   filter;
//! - **a fan-out** and **a tree** hold where their columns, so carried, determine the head's row (its rows are then
//!   at most the body atom's, and a forest's edges kept are a forest);
//! - **an order** holds the same way, dense only where the rule is the relation itself, renamed (a filter leaves
//!   gaps);
//! - **a bound on a column's values** (`values(c, documents | kinds | ~N)`) holds of a head column that copies it;
//!   a share of the rows (`many`, `rows / N`, `within document`) is not carried, as a rule keeps any part of them;
//! - **local** holds where every atom is local, every node the head holds is an atom's, and the atoms that hold
//!   nodes are joined by them (or by `nearest`, which stays in its node's document). A relation of one node column
//!   at most is local by its shape.
//!
//! A relation of several rules, or a recursive one, keeps `local` where every rule gives it (a recursion's supposing
//! its component local, by induction on the rounds), and otherwise its declared facts alone: a key of a union needs its
//! branches apart, and a recursion's other facts a proof (0361: "recursion keeps a fact only when it is declared, or
//! proved by the base case and the step"). An aggregate's, a native's, a request's answer or the program's facts take
//! their declared facts alone.
use super::RelId;
use super::about::About;
use super::analyses::Strata;
use super::handles::{CmpOp, Lit, RuleRef, TermRef};
use super::manager::{Analysis, Cx, Reads};
use super::version::Version;
use crate::code_rule_dsl::logic::front::{LayoutFact, LayoutValues};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Where a relation's fact came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Its `.layout` lines.
    Declared,
    /// Inferred from this rule, its relation's one rule.
    Rule(super::RuleId),
    /// Inferred from every rule of its component (a union's branches, a recursion's rules).
    Rules,
}

/// Each relation's facts, by relation id, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layouts(pub BTreeMap<RelId, Vec<(LayoutFact, Origin)>>);

impl Layouts {
    /// `rel`'s facts.
    pub fn of(&self, rel: RelId) -> impl Iterator<Item = &LayoutFact> + '_ {
        self.0.get(&rel).into_iter().flatten().map(|(f, _)| f)
    }

    /// The facts of `v`'s relations: each relation's declared ones, and, in dependency order, what its one rule gives
    /// from the facts of the relations it reads.
    pub fn infer(v: &Version, about: &About, strata: &Strata) -> Layouts {
        let mut out: BTreeMap<RelId, Vec<(LayoutFact, Origin)>> = about.layouts.iter().map(|(rel, facts)| (*rel, facts.iter().map(|f| (f.clone(), Origin::Declared)).collect())).collect();
        // What is made other than by its rules alone takes its declared facts only.
        let mut other: BTreeSet<RelId> = about.natives.iter().map(|n| n.rel).collect();
        other.extend(about.produced().map(|(p, _)| p));
        other.extend(about.lattice.iter().map(|l| l.rel));
        other.extend(v.all_facts().into_iter().filter_map(|f| v.rel_of(f.name())));
        for scc in &strata.sccs {
            if scc.iter().any(|r| other.contains(r)) {
                continue;
            }
            let recursive = scc.len() > 1 || strata.edges.of_head(scc[0]).any(|(d, _)| d == scc[0]);
            let rules: Vec<RuleRef<'_>> = scc.iter().flat_map(|r| v.heads(*r)).collect();
            if recursive || rules.len() > 1 {
                // A union is local where each branch is; a recursion where each rule is, supposing its component is
                // (by induction on the rounds: the base rules read none of it). No other fact is carried.
                let mut supposed = out.clone();
                for r in scc {
                    supposed.entry(*r).or_default().push((LayoutFact::Local, Origin::Declared));
                }
                if !rules.is_empty() && rules.iter().all(|r| infer_rule(v, *r, &supposed).contains(&LayoutFact::Local)) {
                    for r in scc {
                        let facts = out.entry(*r).or_default();
                        if !facts.iter().any(|(f, _)| *f == LayoutFact::Local) {
                            facts.push((LayoutFact::Local, Origin::Rules));
                        }
                    }
                }
                continue;
            }
            let rel = &scc[0];
            let [rule] = rules.as_slice() else { continue };
            let rule = *rule;
            let inferred = infer_rule(v, rule, &out);
            if inferred.is_empty() {
                continue;
            }
            let facts = out.entry(*rel).or_default();
            for f in inferred {
                if !facts.iter().any(|(g, _)| *g == f) {
                    facts.push((f, Origin::Rule(rule.id())));
                }
            }
        }
        Layouts(out)
    }

    /// The facts as EXPLAIN shows them: one line a relation, each fact with its columns named and where it came from.
    pub fn show(&self, v: &Version) -> String {
        let mut out = String::new();
        for (rel, facts) in &self.0 {
            let name = v.rel_name(*rel);
            let cols: Vec<String> = v.decl_of(name).map(|d| d.cols().map(|(n, _)| v.text(n).to_owned()).collect()).unwrap_or_default();
            let col = |c: usize| cols.get(c).cloned().unwrap_or_else(|| format!("${c}"));
            let shown: Vec<String> = facts
                .iter()
                .map(|(f, o)| match o {
                    Origin::Declared => format!("{} (declared)", f.show(&col)),
                    Origin::Rule(r) => format!("{} (by rule {})", f.show(&col), v.rules().position(|x| x.id() == *r).map_or_else(|| "?".to_owned(), |k| format!("#{k:04}"))),
                    Origin::Rules => format!("{} (by its rules)", f.show(&col)),
                })
                .collect();
            out.push_str(&format!("{name}: {}\n", shown.join(", ")));
        }
        out
    }
}

impl Analysis for Layouts {
    const NAME: &'static str = "layouts";
    const READS: Reads = Reads::RULES.or(Reads::ABOUT);
    fn compute(cx: &Cx<'_>) -> Result<Layouts, String> {
        Ok(Layouts::infer(cx.v(), &*cx.get::<About>()?, &*cx.get::<Strata>()?))
    }
}

/// A variable's slot in a rule.
type Var = super::Slot;

/// The variables of a term, a call's arguments' included.
fn vars_of(t: TermRef<'_>, into: &mut BTreeSet<Var>) {
    match t {
        TermRef::Var(s) => {
            into.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| vars_of(a, into)),
        _ => {}
    }
}

/// What one rule gives its head, from the facts known of the relations its body reads (`known`).
fn infer_rule(v: &Version, r: RuleRef<'_>, known: &BTreeMap<RelId, Vec<(LayoutFact, Origin)>>) -> Vec<LayoutFact> {
    let facts_of = |rel: RelId| known.get(&rel).into_iter().flatten().map(|(f, _)| f);
    let head: Vec<TermRef<'_>> = r.head().args().collect();
    let mut head_vars = BTreeSet::new();
    head.iter().for_each(|t| vars_of(*t, &mut head_vars));
    // Each head variable's first place.
    let mut place: HashMap<Var, usize> = HashMap::new();
    for (k, t) in head.iter().enumerate() {
        if let TermRef::Var(s) = t {
            place.entry(*s).or_insert(k);
        }
    }
    // The positive atoms, and the dependencies: each atom's keys, each equality, each `nearest`.
    let mut atoms: Vec<(RelId, Vec<TermRef<'_>>)> = Vec::new();
    let mut deps: Vec<(BTreeSet<Var>, BTreeSet<Var>)> = Vec::new();
    let mut nearest: Vec<(Var, Var)> = Vec::new();
    let mut only_atom = true;
    for l in r.body() {
        match l.view() {
            Lit::Atom(a) if !a.neg() => atoms.push((a.rel(), a.args().collect())),
            Lit::Atom(_) => only_atom = false,
            Lit::Cmp { l, op: CmpOp::Eq, r: rt } => {
                only_atom = false;
                let (mut lv, mut rv) = (BTreeSet::new(), BTreeSet::new());
                vars_of(l, &mut lv);
                vars_of(rt, &mut rv);
                if let TermRef::Var(x) = l {
                    deps.push((rv.clone(), BTreeSet::from([x])));
                }
                if let TermRef::Var(y) = rt {
                    deps.push((lv, BTreeSet::from([y])));
                }
            }
            Lit::Cmp { .. } => only_atom = false,
            Lit::Nearest { n, a, .. } => {
                only_atom = false;
                deps.push((BTreeSet::from([n]), BTreeSet::from([a])));
                nearest.push((n, a));
            }
            // An aggregate or a matcher makes values no fact describes.
            Lit::Agg { .. } | Lit::Ast { .. } => return Vec::new(),
        }
    }
    let key_vars = |args: &[TermRef<'_>], cols: &[usize]| -> Option<BTreeSet<Var>> {
        let mut ks = BTreeSet::new();
        for &c in cols {
            match args.get(c)? {
                TermRef::Var(s) => {
                    ks.insert(*s);
                }
                TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => {}
                _ => return None,
            }
        }
        Some(ks)
    };
    for (rel, args) in &atoms {
        let mut all = BTreeSet::new();
        args.iter().for_each(|t| vars_of(*t, &mut all));
        for f in facts_of(*rel) {
            if let LayoutFact::Key(cols) = f
                && let Some(ks) = key_vars(args, cols)
            {
                deps.push((ks, all.clone()));
            }
        }
    }
    let closure = |start: &BTreeSet<Var>| -> BTreeSet<Var> {
        let mut got = start.clone();
        loop {
            let before = got.len();
            for (from, to) in &deps {
                if from.is_subset(&got) {
                    got.extend(to.iter().copied());
                }
            }
            if got.len() == before {
                return got;
            }
        }
    };
    // Whether `vs`, all in the head, fix the head's row; and their places there.
    let fixes = |vs: &BTreeSet<Var>| vs.is_subset(&head_vars) && head_vars.is_subset(&closure(vs));
    let places = |vs: &[Var]| -> Option<Vec<usize>> { vs.iter().map(|s| place.get(s).copied()).collect() };
    let one = |args: &[TermRef<'_>], c: usize| args.get(c).and_then(|t| t.var());
    let many = |args: &[TermRef<'_>], cols: &[usize]| -> Option<Vec<Var>> { cols.iter().map(|&c| one(args, c)).collect() };
    let mut out: Vec<LayoutFact> = Vec::new();
    let mut add = |f: LayoutFact| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    // Dense only where the rule is its one atom, each column a variable of its own, renamed.
    let renamed = only_atom && atoms.len() == 1 && {
        let args = &atoms[0].1;
        let vs: BTreeSet<Var> = args.iter().filter_map(|t| t.var()).collect();
        vs.len() == args.len() && head.len() == args.len() && head.iter().all(|t| t.var().is_some())
    };
    for (rel, args) in &atoms {
        for f in facts_of(*rel) {
            match f {
                LayoutFact::Key(cols) => {
                    if let Some(ks) = key_vars(args, cols)
                        && fixes(&ks)
                        && ks.len() < head.len()
                        && let Some(mut at) = places(&ks.iter().copied().collect::<Vec<_>>())
                    {
                        at.sort_unstable();
                        add(LayoutFact::Key(at));
                    }
                }
                LayoutFact::Fanout { from, to, fan } => {
                    if let (Some(fv), Some(tv)) = (many(args, from), many(args, to))
                        && fixes(&fv.iter().chain(&tv).copied().collect())
                        && let (Some(from), Some(to)) = (places(&fv), places(&tv))
                    {
                        add(LayoutFact::Fanout { from, to, fan: *fan });
                    }
                }
                LayoutFact::Tree { from, to } => {
                    if let (Some(fv), Some(tv)) = (one(args, *from), one(args, *to))
                        && fixes(&BTreeSet::from([fv, tv]))
                        && let (Some(&from), Some(&to)) = (place.get(&fv), place.get(&tv))
                    {
                        add(LayoutFact::Tree { from, to });
                    }
                }
                LayoutFact::Order { col, within, dense } => {
                    if let (Some(cv), Some(wv)) = (one(args, *col), many(args, within))
                        && fixes(&wv.iter().copied().chain([cv]).collect())
                        && let (Some(&col), Some(within)) = (place.get(&cv), places(&wv))
                    {
                        add(LayoutFact::Order { col, within, dense: *dense && renamed });
                    }
                }
                // A bound on a column's values holds of any column that copies it: the documents, the kinds, at most
                // about N. A share of the rows (`many`, `rows / N`, `within document`) does not: a rule keeps any part.
                LayoutFact::Values { col, values: values @ (LayoutValues::Documents | LayoutValues::Kinds | LayoutValues::About(_)) } => {
                    if let Some(cv) = one(args, *col)
                        && let Some(&col) = place.get(&cv)
                    {
                        add(LayoutFact::Values { col, values: *values });
                    }
                }
                _ => {}
            }
        }
    }
    if local(v, r.head().rel(), &atoms, &nearest, &head, known) {
        add(LayoutFact::Local);
    }
    out
}

/// Whether a rule's rows are local: every atom is, every node the head holds is an atom's, and the atoms that hold
/// nodes are one group, joined by their shared nodes or by `nearest`.
fn local(v: &Version, head_rel: RelId, atoms: &[(RelId, Vec<TermRef<'_>>)], nearest: &[(Var, Var)], head: &[TermRef<'_>], known: &BTreeMap<RelId, Vec<(LayoutFact, Origin)>>) -> bool {
    let node_col = |rel: RelId, c: usize| v.decl_of(v.rel_name(rel)).and_then(|d| d.cols().nth(c)).is_some_and(|(_, t)| matches!(v.text(t), "node" | "loc"));
    // Local: stated or inferred, or by its shape: a relation of one node column at most crosses no documents.
    let is_local = |rel: RelId| {
        known.get(&rel).is_some_and(|fs| fs.iter().any(|(f, _)| *f == LayoutFact::Local))
            || v.decl_of(v.rel_name(rel)).is_some_and(|d| (0..d.cols().len()).filter(|&c| node_col(rel, c)).count() <= 1)
    };
    if atoms.is_empty() || !atoms.iter().all(|(rel, _)| is_local(*rel)) {
        return false;
    }
    // Each atom's node variables.
    let nodes: Vec<BTreeSet<Var>> = atoms.iter().map(|(rel, args)| args.iter().enumerate().filter(|(c, _)| node_col(*rel, *c)).filter_map(|(_, t)| t.var()).collect()).collect();
    let held: BTreeSet<Var> = nodes.iter().flatten().copied().collect();
    // Each node the head holds (a column of a node type) is an atom's, or the ancestor `nearest` found of one.
    let found = |x: Var| held.contains(&x) || nearest.iter().any(|&(_, a)| a == x);
    if head.iter().enumerate().any(|(c, t)| node_col(head_rel, c) && !t.var().is_some_and(found)) {
        return false;
    }
    // One group: grow from the first atom with nodes through shared node variables and `nearest`.
    let with: Vec<usize> = (0..atoms.len()).filter(|&k| !nodes[k].is_empty()).collect();
    let Some(&first) = with.first() else { return true };
    let mut reached: BTreeSet<Var> = nodes[first].clone();
    let mut joined: BTreeSet<usize> = BTreeSet::from([first]);
    loop {
        let before = joined.len();
        for &(n, a) in nearest {
            if reached.contains(&n) || reached.contains(&a) {
                reached.extend([n, a]);
            }
        }
        for &k in &with {
            if !joined.contains(&k) && !nodes[k].is_disjoint(&reached) {
                joined.insert(k);
                reached.extend(nodes[k].iter().copied());
            }
        }
        if joined.len() == before {
            return joined.len() == with.len();
        }
    }
}

