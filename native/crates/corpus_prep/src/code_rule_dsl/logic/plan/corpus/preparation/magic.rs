//! Demand: magic sets (03 §12 "Demand / magic sets"; 05 Phase 6 row 4), on the version (decision 0281's path, step
//! 6d). A port of Typing-support's `rdl.py` (T) `magic_sets`, `magic_adorn` and `gate_negations`, rule for rule: its
//! transformed programs on T's five programs are the tests' goldens (`magic::tests`).
//!
//! - `.demand r, …` (`KeyDemand`, T's `magic_sets`): each demanded relation restricted to the keys (first column) its
//!   consumers ask for, with magic rules passing as many literals sideways as stay stratifiable.
//! - `.pragma "magic-transform" "r, …"` (`ProgramDemand`, decision 0145): each listed relation adorned over every
//!   column its readers bind, every relation not listed computed in full; `"*"`, or `every`, is T's `magic_adorn`:
//!   Soufflé's transformation on every derived relation, with T's differences (one relation per predicate, key
//!   adornments, sideways order, computed keys, negation kept).
//! - Gated negations (`ProgramDemand::gate`): a negation that demand puts inside its own component waits for its
//!   relation's `done_` gate, which the native engine opens once what the relation reads is quiet.
//!
//! T's other environment switches (`RDL_COARSE`, `RDL_MAGIC_KEY`, `RDL_MAGIC_NONKEY`, `RDL_MAGIC_INPUT_FIRST`) are at
//! T's defaults: magic rules take extras, every bound column is adorned, a non-key column bound by a variable is
//! adorned when the rules answer it, and an input on a bound key goes before a derived atom sideways.
//!
//! Each rule's literals are its own handles; what the transformation adds to a rule is its guard
//! (`magic_<r>_<ad>(…)`), a computed head term's binding (`_m<n> = @f(…)`) and `magic_true(0)`. The variables are a
//! rule's slots, a binding's fresh variable after them. Each program the trials make is a dependency graph, not a
//! program: what `analyses::strata` reads of one.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::logic::front::{Edges, Error, tarjan};
use crate::code_rule_dsl::logic::front::ast::{Gate, Report};
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, AggRef, AtomRef, Cell, CmpOp, Lit, LitRef, Rewrite, Rows, RuleBuilder, RuleRef, Slot, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod split;

/// The profile's topic of each change demand made to keep the program's components (`Report::reasons`): a relation
/// split (`split`), or relations computed in full for a negative cycle.
pub const REASON_TOPIC: &str = "the program's demand: kept apart (decisions 0065 §2a, 0145)";

/// A term of a literal the transformation writes: the rule's own, the fresh variable `_m<n>`, or, in a gate's own
/// rule, its variable `_g<n>`, `_` or `0`.
#[derive(Clone, Copy)]
enum Tm<'v> {
    Own(TermRef<'v>),
    Fresh(u32),
    Gate(u32),
    Wild,
    Zero,
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
    /// The rule's own negation or aggregate, gated: it waits for its relation's `done_` gate.
    Gated(LitRef<'v>),
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
        Tm::Gate(_) | Tm::Wild | Tm::Zero => {}
    }
}

fn is_const(t: Tm<'_>) -> bool {
    matches!(t, Tm::Own(TermRef::Int(_) | TermRef::Str(_)) | Tm::Zero)
}

fn bset(ad: &str) -> BTreeSet<usize> {
    ad.bytes().enumerate().filter(|(_, x)| *x == b'b').map(|(i, _)| i).collect()
}

impl<'v> M<'v> {
    fn vars(&self, r: RuleRef<'v>) -> VarSet {
        let mut out = VarSet::new();
        match self {
            M::Own(l) | M::Gated(l) => return lit_vars(*l),
            M::Guard(_, args) => args.iter().for_each(|t| tm_vars(r, *t, &mut out)),
            M::Call(n, call) => {
                out.insert(fresh(r, *n));
                term_vars(*call, &mut out);
            }
            M::True => {}
        }
        out
    }
    /// A positive atom: its relation.
    fn positive(&self, v: &'v Version) -> Option<&str> {
        match self {
            M::Own(l) => match l.view() {
                Lit::Atom(a) if !a.neg() => Some(v.rel_name(a.rel())),
                _ => None,
            },
            M::Guard(rel, _) => Some(rel),
            M::True => Some("magic_true"),
            M::Call(..) | M::Gated(_) => None,
        }
    }
    /// The first term of a positive atom.
    fn first(&self) -> Option<Tm<'v>> {
        match self {
            M::Own(l) => l.atom().and_then(|a| a.args().next()).map(Tm::Own),
            M::Guard(_, args) => args.first().copied(),
            M::True | M::Call(..) | M::Gated(_) => None,
        }
    }
    /// The atom it reads: its relation, whether negated or aggregated.
    fn atom(&self, v: &'v Version) -> Option<(&str, bool, bool)> {
        match self {
            M::Own(l) | M::Gated(l) => match l.view() {
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
        matches!(self, M::Own(l) | M::Gated(l) if matches!(l.view(), Lit::Agg { .. }))
    }
    fn gated(&self) -> bool {
        match self {
            M::Own(l) => l.atom().is_some_and(|a| a.gated()),
            M::Gated(_) => true,
            _ => false,
        }
    }
    fn show(&self, v: &Version, r: RuleRef<'_>) -> String {
        match self {
            M::Own(l) | M::Gated(l) => v.show_lit(r, *l),
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
        Tm::Gate(n) => format!("$_g{n}"),
        Tm::Wild => "_".into(),
        Tm::Zero => "0".into(),
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

/// The variables literal `l` binds once it runs, or `None` if it cannot run yet (the planner's `ready`).
fn binds(v: &Version, r: RuleRef<'_>, l: &M<'_>, bound: &VarSet) -> Option<VarSet> {
    match l {
        M::Own(x) => match x.view() {
            Lit::Atom(a) if !a.neg() => Some(atom_vars(a)),
            Lit::Atom(a) => atom_vars(a).is_subset(bound).then(VarSet::new),
            Lit::Cmp { l, op, r: rt } => cmp_binds(v, r, Tm::Own(l), op, rt, bound),
            Lit::Agg { var, op, over, atom } => {
                // The aggregate's own variables, as in the planner's `ready`: the aggregated one and join's order.
                let mut need = atom_vars(atom);
                need.remove(var);
                if let Some(o) = over {
                    need.remove(o);
                }
                if let AggRef::Join { by, .. } = op {
                    by.for_each(|k| need.remove(k));
                }
                if let Some(b) = op.bound() {
                    need.insert(b);
                }
                need.is_subset(bound).then(|| VarSet::of([var]))
            }
            Lit::Nearest { n, a, .. } => (bound.contains(n) || bound.contains(a)).then(|| lit_vars(*x)),
            Lit::Ast { .. } => Some(lit_vars(*x)),
        },
        M::Guard(..) | M::True => Some(l.vars(r)),
        M::Call(n, call) => cmp_binds(v, r, Tm::Fresh(*n), CmpOp::Eq, *call, bound),
        M::Gated(x) => binds(v, r, &M::Own(*x), bound),
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

/// A failure of one attempt: a negative cycle that demand closed (with the relations negated or aggregated inside
/// it), or any other error.
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
    /// `b` for each bound argument; all free unless the key is bound (demand on later columns alone restricts little
    /// and reverses the rules' direction), or a non-key column is bound by a variable and the rules answer that
    /// adornment.
    fn adorn(&mut self, a: AtomRef<'_>, bound: &VarSet, full: &BTreeSet<String>) -> String {
        let ad = self.pattern(a, bound);
        if !ad.starts_with('b') {
            let free = "f".repeat(ad.len());
            return if Self::var_bound(a, &ad) && self.answers(self.v.rel_name(a.rel()), &ad, &[], full) { ad } else { free };
        }
        ad
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
    /// Whether `rel`'s column `i` holds a node.
    fn node_col(&self, rel: &str, i: usize) -> bool {
        self.v.decl_of(rel).and_then(|d| d.cols().nth(i).map(|(_, ty)| self.v.text(ty) == "node")).unwrap_or(false)
    }
    /// Whether `l` ties nodes: a positive atom, a guard, `nearest` or an AST match.
    fn ties(&self, l: &M<'v>) -> bool {
        match l {
            M::Own(x) => matches!(x.view(), Lit::Atom(a) if !a.neg()) || matches!(x.view(), Lit::Nearest { .. } | Lit::Ast { .. }),
            M::Guard(..) => true,
            _ => false,
        }
    }
    /// The variables a rule's literals hold as a document's id: `docof(_, d)`, `doc(d, …)`, `node(d, …)`.
    fn doc_vars(&self, pre: &[(M<'v>, bool)]) -> VarSet {
        let mut out = VarSet::new();
        for (l, _) in pre {
            if let M::Own(x) = l
                && let Lit::Atom(a) = x.view()
                && !a.neg()
            {
                let col = match self.v.rel_name(a.rel()) {
                    "docof" => 1,
                    "doc" | "node" => 0,
                    _ => continue,
                };
                if let Some(TermRef::Var(s)) = a.args().nth(col) {
                    out.insert(s);
                }
            }
        }
        out
    }
    /// The variables literal `l` holds in node columns or as a document's id (`docs`): a positive atom's, a guard's
    /// (its relation's bound columns), `nearest`'s two, an AST match's node and document.
    fn node_vars(&self, r: RuleRef<'v>, l: &M<'v>, docs: &VarSet) -> VarSet {
        let mut out = VarSet::new();
        match l {
            M::Own(x) => match x.view() {
                Lit::Atom(a) if !a.neg() => {
                    let rel = self.v.rel_name(a.rel());
                    for (i, t) in a.args().enumerate() {
                        if let TermRef::Var(s) = t
                            && (self.node_col(rel, i) || docs.contains(s))
                        {
                            out.insert(s);
                        }
                    }
                }
                Lit::Nearest { n, a, .. } => {
                    out.insert(n);
                    out.insert(a);
                }
                Lit::Ast { document, node, .. } => {
                    out.insert(node);
                    out.insert(document);
                }
                _ => {}
            },
            M::Guard(name, args) => {
                if let Some((rel, ad)) = name.strip_prefix("magic_").and_then(|n| n.rsplit_once('_')) {
                    let cols: Vec<usize> = ad.bytes().enumerate().filter(|(_, x)| *x == b'b').map(|(i, _)| i).collect();
                    for (t, &i) in args.iter().zip(&cols) {
                        if self.node_col(rel, i) {
                            tm_vars(r, *t, &mut out);
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }
    /// A magic rule's sideways literals for a relation whose bound columns are all nodes (`unparen_bf`, a node's own
    /// rows): those tied to its arguments through node columns, and the filters over their variables. A node's demand
    /// is its own document's, so a literal reached only through a value (a name, a symbol) joins another document's
    /// rows to it and asks nothing more of this one: default-called-once's `magic_unparen_bf(v)` joined every call's
    /// callee name to every field's class name (`t`), a crossing the split refuses (decision 0695). Leaving a literal
    /// out only widens the demand. Unchanged when the tied literals would not bind the arguments.
    fn tied(&self, r: RuleRef<'v>, rel: &str, ad: &str, args: &[Tm<'v>], pre: &[(M<'v>, bool)]) -> Vec<(M<'v>, bool)> {
        let bound_cols: Vec<usize> = ad.bytes().enumerate().filter(|(_, x)| *x == b'b').map(|(i, _)| i).collect();
        if bound_cols.is_empty() || !bound_cols.iter().all(|&i| self.node_col(rel, i)) {
            return pre.to_vec();
        }
        let mut seeds = VarSet::new();
        args.iter().for_each(|t| tm_vars(r, *t, &mut seeds));
        let want = seeds.clone();
        let docs = self.doc_vars(pre);
        let mut taken = vec![false; pre.len()];
        loop {
            let mut more = false;
            for (i, (l, _)) in pre.iter().enumerate() {
                if taken[i] || !self.ties(l) {
                    continue;
                }
                let nv = self.node_vars(r, l, &docs);
                if !nv.is_disjoint(&seeds) {
                    taken[i] = true;
                    seeds.extend(&nv);
                    more = true;
                }
            }
            if !more {
                break;
            }
        }
        let mut held = VarSet::new();
        for (i, (l, _)) in pre.iter().enumerate() {
            if taken[i] {
                held.extend(&l.vars(r));
            }
        }
        if !want.is_subset(&held) {
            return pre.to_vec();
        }
        // An atom over variables the tied literals hold only tests them (`seqkind(kl)`), and so does one whose other
        // variables no other literal reads (`rifrom(s, x, m, y)` for x): it is kept.
        let others = |i: usize| {
            let mut o = VarSet::new();
            for (j, (l, _)) in pre.iter().enumerate() {
                if j != i && !taken[j] {
                    o.extend(&l.vars(r));
                }
            }
            o
        };
        let tests: Vec<usize> = (0..pre.len())
            .filter(|&i| {
                let vars = pre[i].0.vars(r);
                !taken[i] && self.ties(&pre[i].0) && (vars.is_subset(&held) || (!vars.is_disjoint(&held) && vars.minus(&held).is_disjoint(&others(i))))
            })
            .collect();
        for i in tests {
            taken[i] = true;
        }
        let mut left = VarSet::new();
        for (i, (l, _)) in pre.iter().enumerate() {
            if !taken[i] && self.ties(l) {
                left.extend(&l.vars(r));
            }
        }
        // A filter goes with the literals it reads: out when a variable of it is bound only by one left out.
        let gone = left.minus(&held);
        pre.iter().enumerate().filter(|(i, (l, _))| taken[*i] || (!self.ties(l) && l.vars(r).is_disjoint(&gone))).map(|(_, x)| x.clone()).collect()
    }
    /// The body in sideways order: filters as soon as they can run; then the left-most positive atom whose key is
    /// bound, an input before a derived one; else one sharing a bound variable (an input or view first); else the
    /// lowest one in the strata (inputs first). `None` if no order binds the rest.
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
            let pos: Vector<usize> = (0..rest.len()).filter(|&i| rest[i].positive(v).is_some()).collect();
            if pos.is_empty() {
                return None;
            }
            // A nullary atom is an existence test: it runs with no bindings.
            let first = |i: usize| match rest[i].first() {
                Some(t) if is_const(t) => true,
                Some(Tm::Own(TermRef::Var(x))) => bound.contains(x),
                Some(Tm::Fresh(n)) => bound.contains(fresh(r, n)),
                Some(_) => false,
                // A nullary atom is an existence test: it runs with no bindings (ported from cost-classifier-metal-rf's
                // fix to the tree magic, `a_nullary_guard_is_ready_in_a_demanded_body`).
                None => !matches!(rest[i], M::True),
            };
            let derived = |i: usize| self.idb.contains(rest[i].positive(v).expect("a positive atom"));
            let joined: Vector<usize> = pos.iter().copied().filter(|&i| !rest[i].vars(r).is_disjoint(&bound)).collect();
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
                    // A computed head term at a bound position is matched through a fresh variable: the guard binds
                    // it, and `$m = @cat($q, $x)` tests it once the body binds the call's arguments.
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
                        // An aggregated relation is computed in full: its demand would pull the count into its
                        // consumers' recursive component.
                        let ad2 = if full.contains(arel) || l.is_agg() { "f".repeat(self.arity(arel)) } else { self.adorn(a, &bound, full) };
                        let ad2 = coarse.get(&(arel.to_owned(), ad2.clone())).cloned().unwrap_or(ad2);
                        if ad2.contains('b') {
                            let args: Vec<Tm<'_>> = a.args().zip(ad2.bytes()).filter(|(_, x)| *x == b'b').map(|(t, _)| Tm::Own(t)).collect();
                            let pre = self.tied(rule, arel, &ad2, &args, &pre);
                            magic.push(((format!("magic_{arel}_{ad2}"), args), rule, pre));
                        }
                        todo.push((arel.to_owned(), ad2));
                    }
                    // Sideways: input atoms and filters always; derived atoms and negations as extras, kept if the
                    // program stays stratifiable. Leaving one out only widens the demand.
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

/// A trial's program's dependency graph, stratified (`analyses::strata`): its components, dependencies first, and its
/// edges by name.
fn strata(v: &Version, about: &About, rules: &[Made<'_>]) -> Result<(crate::code_rule_dsl::logic::front::Components, Edges), Error> {
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

/// The program's own demand, a corpus run's `.pragma "magic-transform" "r, …"` (decision 0145, "Building on it"):
/// each listed relation computed only for what its readers ask, by adornments over every column they bind
/// (`found(c, a, at)` read with `c` and `a` bound is `found_bbf`), not its first column alone as `KeyDemand` (T's
/// `.demand`) restricts it. It is T's `magic_adorn` with every relation not listed computed in full; `"*"` (or
/// `every`, T's `.demand *`) is `magic_adorn` itself: Soufflé's magic-set transformation on every derived relation. A
/// use of a demanded relation with nothing bound computes it in full, and a negative cycle demand would close makes
/// each relation negated inside it, with what it depends on, computed in full, and the transformation is tried again:
/// leaving demand out only widens it, so the rows are the program's. Its report, and the rules it wrote; `None`
/// where the program lists none.
#[derive(Default)]
pub struct ProgramDemand {
    /// Every derived relation demanded, whatever the program lists (`"*"`, `.demand *`: T's `magic_adorn` itself).
    pub every: bool,
    /// Gated negations (T's `RDL_GATE`, with the native program mode): a negation that demand puts inside its own
    /// component waits for its relation's `done_` gate, which the native engine opens once what the relation reads
    /// is quiet, instead of the relations negated there computed in full.
    pub gate: bool,
    /// The relations listed, and what became of them: the transformation's report, or why it was not applied (the
    /// program is then as it was, every relation computed in full).
    pub listed: Vec<String>,
    pub outcome: Option<std::result::Result<Report, String>>,
    /// Each relation split (`split`) and its copy, in the order made.
    pub splits: Vec<(String, String)>,
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

/// The rounds of splits (`split`) one program may take: each splits what the last left, one relation below it.
const SPLIT_ROUNDS: usize = 32;

impl ProgramDemand {
    /// The transformation (`transform`), and while its demand joins a request's component with what the request reads
    /// (`split::joins`), each such demand split and the program transformed again. Each split is a decision of the
    /// run's profile: the relation, the cycle with the rule of each step, and the rules moved to the copy.
    fn apply(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> std::result::Result<(), String> {
        let mut owned: Option<Arc<Version>> = None;
        let (mut copies, mut reasons): (Vec<(String, String)>, Vec<String>) = (Vec::new(), Vec::new());
        for round in 0..=SPLIT_ROUNDS {
            let v: &Version = owned.as_deref().unwrap_or_else(|| cx.v());
            let about = About::of(v);
            let Some((trial, decls, mut report)) = self.transform(v, &about, &copies)? else { return Ok(()) };
            let requests = split::requests(v, &about);
            let program = split::graph(v, &requests);
            let found = split::joins(v, &about, &trial.made, &program, &requests);
            if found.is_empty() {
                self.splits = copies;
                reasons.append(&mut report.reasons);
                report.reasons = reasons;
                return self.write(v, w, trial, decls, report);
            }
            if round == SPLIT_ROUNDS {
                return Err(format!("demand still joins a request's component after {SPLIT_ROUNDS} rounds of splits: {}", found[0].why));
            }
            reasons.extend(found.iter().map(|s| s.why.clone()));
            let (next, made) = split::split(v, &found, &program)?;
            copies.extend(made);
            owned = Some(next);
        }
        unreachable!("a round returns")
    }

    /// The transformation of `v`, with the relations of `copies` (each a relation's copy) demanded as theirs are: the
    /// program, the guards' declarations and the report; `None` where the program lists nothing.
    #[allow(clippy::type_complexity)]
    fn transform<'v>(&mut self, v: &'v Version, about: &About, copies: &[(String, String)]) -> std::result::Result<Option<(Trial<'v>, Vec<(String, Vec<(String, String)>)>, Report)>, String> {
        let mut listed: Vec<&str> = about.demand_list.iter().map(|a| v.text(*a)).collect();
        if listed.is_empty() && !self.every {
            return Ok(None);
        }
        self.listed = listed.iter().map(|x| (*x).to_owned()).collect();
        listed.extend(copies.iter().filter(|(of, _)| listed.contains(&of.as_str())).map(|(_, c)| c.as_str()).collect::<Vec<_>>());
        let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let mut full: BTreeSet<String> = BTreeSet::new();
        if !self.every && !listed.contains(&"*") {
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
        // A relation that aggregates is computed in full: guarded, its count would join its demand's recursive component.
        full.extend(v.rules().filter(|r| r.body().any(|l| matches!(l.view(), Lit::Agg { .. }))).map(|r| v.rel_name(r.head().rel()).to_owned()));
        let originals: Vec<Made<'_>> = v.rules().map(|rule| Made { rule, head: None, body: rule.body().map(M::Own).collect() }).collect();
        let (sccs, _) = strata(v, about, &originals).map_err(|e| e.to_string())?;
        let level: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.to_owned(), i))).collect();
        let (mut cyc, mut why) = (Vec::new(), Vec::new());
        let made = loop {
            let mut cx2 = Ctx { v, about, by: &by, idb: &idb, level: &level, answered: BTreeMap::new() };
            match attempt(&mut cx2, &full, self.gate) {
                Ok(made) => break made,
                Err(Attempt::Failed(e)) => return Err(e.to_string()),
                Err(Attempt::Cycle { error, negs }) => {
                    // Each relation negated inside the component is computed in full, with every relation it depends
                    // on, so that its demand no longer comes from its consumers.
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
                    why.push(format!("{} computed in full, with what they read ({}): {error}", negs.iter().cloned().collect::<Vec<_>>().join(", "), new.iter().cloned().collect::<Vec<_>>().join(", ")));
                    cyc.extend(negs.intersection(&new).cloned());
                    full.extend(new);
                }
            }
        };
        let (trial, decls, mut report) = made;
        report.full_for_negation = cyc;
        report.reasons = why;
        Ok(Some((trial, decls, report)))
    }

    /// The program as the attempt made it, over `v` (the input, or its split): its rules, the guards' and the copies'
    /// declarations, `magic_true`, and no demand left.
    fn write(&mut self, v: &Version, w: &mut ProgramW<'_>, trial: Trial<'_>, decls: Vec<(String, Vec<(String, String)>)>, report: Report) -> std::result::Result<(), String> {
        let input = w.input().clone();
        let b = w.derive()?;
        for m in &trial.made {
            write(v, b.rule(), m)?;
        }
        for (_, copy) in &self.splits {
            if let Some(d) = v.decl_of(copy) {
                let cols: Vec<(String, String)> = d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
                let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                b.fields().declare(copy, &cols);
            }
        }
        for (rel, cols) in decls.iter().chain(&trial.done) {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(rel, &cols);
        }
        b.fields().declare("magic_true", &[("k", "int")]);
        let mut rows = input.find("magic_true").and_then(|a| input.facts(a)).map(|f| Rows::of_cells(f.rows())).unwrap_or_default();
        rows.push([Cell::Int(0)]);
        b.fields().set_fact_cells("magic_true", rows);
        // No demand left, and only the gates this demand made.
        for name in ["demand", "gate", "gate_src", "gate_quiet", "gate_dep"] {
            if !b.about_rows(name).is_empty() {
                b.fields().set_about_rows(name, Rows::new());
            }
        }
        let pc = super::demand::about_p(b);
        // The gates, as the program keeps them (`gate`, `gate_src`, `gate_quiet`, `gate_dep`).
        let (mut gate, mut src, mut quiet, mut deps) = (Rows::new(), Rows::new(), Rows::new(), Rows::new());
        for (k, (key, g)) in trial.gates.iter().enumerate() {
            let (key, rel) = (b.atom(key), b.atom(&g.rel));
            gate.push([pc, Cell::Int(k as i64), Cell::Str(key), Cell::Str(rel), Cell::Int(g.level as i64)]);
            if let Some(s) = &g.src {
                src.push([pc, Cell::Int(k as i64), Cell::Str(b.atom(s))]);
            }
            for (out, items) in [(&mut quiet, &g.quiet), (&mut deps, &g.deps)] {
                for (i, rel) in items.iter().enumerate() {
                    out.push([pc, Cell::Int(k as i64), Cell::Int(i as i64), Cell::Str(b.atom(rel))]);
                }
            }
        }
        for (name, rows) in [("gate", gate), ("gate_src", src), ("gate_quiet", quiet), ("gate_dep", deps)] {
            if !rows.is_empty() {
                b.fields().set_about_rows(name, rows);
            }
        }
        // The report, as the program keeps it (`Program::magic_report`: `magic_report` and `report_list`).
        b.fields().set_about_rows("magic_report", Rows::from_iter([[pc, Cell::Int(report.magic_rules as i64), Cell::Int(report.extras_kept as i64), Cell::Int(report.gates as i64)]]));
        let mut list = Rows::new();
        for (name, items) in [("adornments", &report.adornments), ("full", &report.full), ("full_for_negation", &report.full_for_negation), ("reasons", &report.reasons)] {
            for (k, item) in items.iter().enumerate() {
                let (n, x) = (b.atom(name), b.atom(item));
                list.push([pc, Cell::Str(n), Cell::Int(k as i64), Cell::Str(x)]);
            }
        }
        b.fields().set_about_rows("report_list", list);
        self.outcome = Some(Ok(report));
        Ok(())
    }
}

/// `.demand r, …` (T's `magic_sets`): each demanded relation restricted to the keys (first column) its consumers ask
/// for. Every rule for a demanded R gets `magic_R(key)` first; every use of a demanded S with its key bound produces
/// `magic_S(key) :- <the literals before it>`, a negated use too. Extras (negations, and atoms of the head's own
/// recursive component) are added greedily while the program stays stratifiable; leaving one out only widens demand.
/// A demanded relation whose negated use would close a negative cycle through its own demand is dropped, computed in
/// full (`dropped` says why), and so is one a drop leaves read with an unbound key. Per wave (C10, list step 21),
/// `waves::policies` demands each lowered `ast` atom by document, and the magic rules decide which (clause, document)
/// pairs the chunk's batch runs.
#[derive(Default)]
pub struct KeyDemand {
    /// The relations computed in full, and why.
    pub dropped: Vec<(String, String)>,
    /// The extras the magic rules kept.
    pub extras: Option<usize>,
}

impl ProgramPass for KeyDemand {
    const NAME: &'static str = "the keys' demand";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> std::result::Result<(), String> {
        let v = cx.v();
        let about = About::of(v);
        let listed: Vec<String> = about.demand_list.iter().map(|a| v.text(*a).to_owned()).collect();
        if listed.is_empty() {
            return Ok(());
        }
        let mut dropped: Vec<(String, String)> = Vec::new();
        let (demand, made) = loop {
            let demand: Vec<String> = listed.iter().filter(|r| !dropped.iter().any(|(d, _)| d == *r)).cloned().collect();
            match keys_once(v, &about, &demand) {
                Ok(made) => break (demand, made),
                Err(e) => {
                    if let Some((_, negated)) = e.unstratified() {
                        if !demand.iter().any(|d| d == negated) {
                            return Err(e.to_string());
                        }
                        dropped.push((negated.to_owned(), "negative cycle through its demand".to_owned()));
                    } else if let Error::UnboundDemand { relation, .. } = &e {
                        // Only a consequence of an earlier drop; otherwise the program asks for unbounded demand.
                        let Some((last, _)) = dropped.last() else { return Err(e.to_string()) };
                        let why = format!("a use with an unbound key once {last} is computed in full");
                        dropped.push((relation.clone(), why));
                    } else {
                        return Err(e.to_string());
                    }
                }
            }
        };
        self.dropped = dropped.clone();
        let input = w.input().clone();
        let b = w.derive()?;
        match &made {
            Some((rules, decls, extras)) => {
                for m in rules {
                    write(&input, b.rule(), m)?;
                }
                for (rel, cols) in decls {
                    let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                    b.fields().declare(rel, &cols);
                }
                b.fields().declare("magic_true", &[("k", "int")]);
                let mut rows = input.find("magic_true").and_then(|a| input.facts(a)).map(|f| Rows::of_cells(f.rows())).unwrap_or_default();
                rows.push([Cell::Int(0)]);
                b.fields().set_fact_cells("magic_true", rows);
                self.extras = Some(*extras);
            }
            None => {
                for r in input.rules() {
                    b.keep(&input, r);
                }
            }
        }
        // The demand left, what was dropped and the extras kept, as the program keeps them.
        let pc = super::demand::about_p(b);
        let rows: Rows = demand.iter().enumerate().map(|(k, r)| [pc, Cell::Int(k as i64), Cell::Str(b.atom(r))]).collect();
        b.fields().set_about_rows("demand", rows);
        let rows: Rows = dropped.iter().enumerate().map(|(k, (r, why))| [pc, Cell::Int(k as i64), Cell::Str(b.atom(r)), Cell::Str(b.atom(why))]).collect();
        b.fields().set_about_rows("demand_dropped", rows);
        if let Some(n) = self.extras {
            b.fields().set_about_rows("magic_extras", Rows::from_iter([[pc, Cell::Int(n as i64)]]));
        }
        Ok(())
    }
}

/// One attempt of `KeyDemand` with `demand` demanded (T's `magic_sets_once`): the rules, the magic relations'
/// declarations and the extras kept; `None` with nothing demanded.
#[allow(clippy::type_complexity)]
fn keys_once<'v>(v: &'v Version, about: &About, demand: &[String]) -> Result<Option<(Vec<Made<'v>>, Vec<(String, Vec<(String, String)>)>, usize)>, Error> {
    if demand.is_empty() {
        return Ok(None);
    }
    let originals: Vec<Made<'_>> = v.rules().map(|rule| Made { rule, head: None, body: rule.body().map(M::Own).collect() }).collect();
    let (sccs, _) = strata(v, about, &originals)?;
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r, i))).collect();
    let demanded = |r: &str| demand.iter().any(|d| d == r);
    let (mut rules, mut magic): (Vec<Made<'v>>, Magic<'v>) = (Vec::new(), Vec::new());
    for rule in v.rules() {
        let h = v.rel_name(rule.head().rel());
        let (mut seen, mut bound, mut body) = (Vec::<(M<'v>, bool)>::new(), VarSet::new(), Vec::new());
        if demanded(h) {
            let key = rule.head().args().next().ok_or_else(|| Error::Syntax(format!("{h} is demanded and has no key")))?;
            term_vars(key, &mut bound);
            let g = M::Guard(format!("magic_{h}"), vec![Tm::Own(key)]);
            seen.push((g.clone(), false));
            body.push(g);
        }
        for l in rule.body() {
            if let Lit::Atom(a) = l.view()
                && demanded(v.rel_name(a.rel()))
            {
                let rel = v.rel_name(a.rel());
                let key = a.args().next().ok_or_else(|| Error::Syntax(format!("{rel} is demanded and has no key")))?;
                let mut kv = VarSet::new();
                term_vars(key, &mut kv);
                if !(kv.is_subset(&bound) || is_const(Tm::Own(key))) {
                    return Err(Error::UnboundDemand {
                        message: format!("demand for {rel} is unbound in the rule for {h}: [{}]", rule.body().map(|l| v.show_lit(rule, l)).collect::<Vec<_>>().join(", ")),
                        relation: rel.to_owned(),
                    });
                }
                magic.push(((format!("magic_{rel}"), vec![Tm::Own(key)]), rule, seen.clone()));
            }
            match l.view() {
                Lit::Atom(a) if !a.neg() => {
                    let rel = v.rel_name(a.rel());
                    let same = demanded(rel) && comp.get(rel) == comp.get(h);
                    seen.push((M::Own(l), same));
                    bound.extend(&atom_vars(a));
                }
                Lit::Atom(a) if atom_vars(a).is_subset(&bound) => seen.push((M::Own(l), true)),
                Lit::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a) => {
                    seen.push((M::Own(l), false));
                    bound.extend(&lit_vars(l));
                }
                Lit::Cmp { .. } if lit_vars(l).is_subset(&bound) => seen.push((M::Own(l), false)),
                _ => {}
            }
        }
        body.extend(rule.body().map(M::Own));
        rules.push(Made { rule, head: None, body });
    }
    let mut decls = Vec::new();
    for r in demand {
        let d = v.decl_of(r).ok_or_else(|| Error::Syntax(format!("{r} is not declared")))?;
        let (_, ty) = d.cols().next().ok_or_else(|| Error::Syntax(format!("{r} has no columns")))?;
        decls.push((format!("magic_{r}"), vec![("k".to_owned(), v.text(ty).to_owned())]));
    }
    let build = |keep: &[Vec<bool>]| -> Result<Vec<Made<'v>>, Error> {
        let mut out = Vec::new();
        for ((mh, r, pre), ks) in magic.iter().zip(keep) {
            // A kept literal still needs its variables bound by the kept literals before it.
            let (mut bound, mut body) = (VarSet::new(), Vec::new());
            for ((l, _), k) in pre.iter().zip(ks) {
                if !k {
                    continue;
                }
                let nearest = matches!(l, M::Own(x) if matches!(x.view(), Lit::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a)));
                if l.positive(v).is_some() || nearest {
                    body.push(l.clone());
                    bound.extend(&l.vars(*r));
                } else if l.vars(*r).is_subset(&bound) {
                    body.push(l.clone());
                }
            }
            let mut key = VarSet::new();
            tm_vars(*r, mh.1[0], &mut key);
            if !key.is_subset(&bound) && !is_const(mh.1[0]) {
                let rel = &mh.0["magic_".len()..];
                return Err(Error::UnboundDemand {
                    message: format!("demand for {rel} is unbound: [{}]", pre.iter().map(|p| p.0.show(v, *r)).collect::<Vec<_>>().join(", ")),
                    relation: rel.to_owned(),
                });
            }
            if body.is_empty() {
                body.push(M::True);
            }
            out.push(Made { rule: *r, head: Some(mh.clone()), body });
        }
        out.extend(rules.iter().cloned());
        Ok(out)
    };
    // Greedy: start from no extras (always stratifiable) and keep each extra that leaves the program stratifiable.
    let mut keep: Vec<Vec<bool>> = magic.iter().map(|(_, _, pre)| pre.iter().map(|(_, extra)| !extra).collect()).collect();
    strata(v, about, &build(&keep)?)?;
    for i in 0..magic.len() {
        for j in 0..magic[i].2.len() {
            if !magic[i].2[j].1 {
                continue;
            }
            keep[i][j] = true;
            match strata(v, about, &build(&keep)?) {
                Ok(_) => {}
                Err(e) if e.unstratified().is_some() => keep[i][j] = false,
                Err(e) => return Err(e),
            }
        }
    }
    let extras = magic.iter().zip(&keep).map(|((_, _, pre), ks)| pre.iter().zip(ks).filter(|((_, extra), k)| *extra && **k).count()).sum();
    Ok(Some((build(&keep)?, decls, extras)))
}

/// Leaves out of a magic rule's body (`ks`) each literal the key does not need that the literals left out had tied to
/// it, and that now joins it by a value alone. A literal's places are its variables at a `node`, `loc` or `span`
/// column, or a document's (`document_column`), anywhere in rule `r`. Counting every literal of the body, its places
/// reach the key's; counting the kept ones, none do. Left in, it joins another document's nodes to the key's by a
/// value, and makes the demand of a relation of one file read two (decision 0677):
/// `magic_unparen_bf(c) :- same_child(x, i, c), same_child(y, i, d)` was RUF034's, `same_pair(x, y)` left out (an
/// extra that would unstratify the program), y's file joined to c's by the index i. A literal the program itself
/// joins by a value (`magic_lookup_fbb(x, m)` beside a scope s) is the author's, and stays. Leaving a literal out
/// only widens the demand. A key with no place is left as it is.
///
/// The same holds of a literal the key's binders need only through a value, where the rule ties it to the key through
/// the demanded atom itself, or a literal after it. A demand answered through a coarser one (`unparen_bb` through
/// `unparen_bf`) keeps the prefix that bound the dropped column: default-called-once's
/// `magic_unparen_bf(v) :- field(c, "function", e), kind(c, "call"), sym_of(e, q), dco_w161_whole(t, q),
/// dco_field(f, t), call_arg(f, -1, "default", v)` joined the call c's file to v's by t, c tied to v by the demanded
/// `unparen(v, c)` alone. Such a literal is left out too unless it reads a key variable, and only while the positive
/// literals kept still bind the key.
fn drop_other_documents<'v>(v: &Version, r: RuleRef<'v>, key: &VarSet, pre: &[(M<'v>, bool)], needed: &[bool], ks: &mut [bool]) {
    let mut places = VarSet::new();
    let mut place = |a: AtomRef<'_>| {
        let rel = v.rel_name(a.rel());
        let Some(d) = v.decl_of(rel) else { return };
        let document = crate::code_rule_dsl::logic::front::document_column(rel);
        for (i, (t, (_, ty))) in a.args().zip(d.cols()).enumerate() {
            if let TermRef::Var(x) = t
                && (matches!(v.text(ty), "node" | "loc" | "span") || document == Some(i))
            {
                places.insert(x);
            }
        }
    };
    place(r.head());
    for l in r.body() {
        if let Lit::Atom(a) | Lit::Agg { atom: a, .. } = l.view() {
            place(a);
        }
    }
    let only_places = |vars: VarSet| vars.minus(&vars.minus(&places));
    let placed: Vec<VarSet> = pre.iter().map(|(l, _)| only_places(l.vars(r))).collect();
    // The places of each literal of the rule: the demanded atom's and those after it too.
    let whole: Vec<VarSet> = r.body().map(|l| only_places(lit_vars(l))).collect();
    let from = key.minus(&key.minus(&places));
    if from.is_empty() {
        return;
    }
    // The places the key reaches through the place sets `with` keeps.
    let reach = |sets: &[VarSet], with: &dyn Fn(usize) -> bool| {
        let mut reach = from.clone();
        loop {
            let mut grew = false;
            for (i, p) in sets.iter().enumerate() {
                if with(i) && !p.is_disjoint(&reach) && !p.is_subset(&reach) {
                    reach.extend(p);
                    grew = true;
                }
            }
            if !grew {
                return reach;
            }
        }
    };
    let (every, kept) = (reach(&whole, &|_| true), reach(&placed, &|i| ks[i]));
    let before = ks.to_vec();
    for (i, p) in placed.iter().enumerate() {
        let reads_key = !pre[i].0.vars(r).is_disjoint(key);
        if ks[i] && (!needed[i] || !reads_key) && !p.is_empty() && !p.is_disjoint(&every) && p.is_disjoint(&kept) {
            ks[i] = false;
        }
    }
    // The positive literals kept must still bind the key; otherwise the demand stays as it was.
    let mut bound = VarSet::new();
    for (i, (l, _)) in pre.iter().enumerate() {
        if ks[i] && l.positive(v).is_some() {
            bound.extend(&l.vars(r));
        }
    }
    if !key.is_subset(&bound) {
        ks.copy_from_slice(&before);
    }
}

/// A trial's program: its rules, and with `gate`, the gates its negations wait for and the gates' declarations.
#[derive(Default)]
struct Trial<'v> {
    made: Vec<Made<'v>>,
    gates: Vec<(String, Gate)>,
    done: Vec<(String, Vec<(String, String)>)>,
}

/// One attempt with `full` computed in full (T's `magic_adorn_once`): the program, the guards' declarations and the
/// report. With `gate`, a negation demand puts inside its own component is gated (`gate_negations`), not a cycle.
#[allow(clippy::type_complexity)]
fn attempt<'v>(cx: &mut Ctx<'v, '_>, full_in: &BTreeSet<String>, gate: bool) -> Result<(Trial<'v>, Vec<(String, Vec<(String, String)>)>, Report), Attempt> {
    let v = cx.v;
    // A relation demanded with every argument free anywhere is computed in full; an adornment binding a superset of
    // another's columns is answered through the other (`pv_bbb` through `pv_bbf`).
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
        // A left-out literal that binds a key variable, or what such a literal needs, stays.
        let (mut ks, mut need) = (ks.to_vec(), key.clone());
        let mut needed = vec![false; pre.len()];
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
                // A gated negation gives nothing (and gates come after the magic rules' prefixes).
                M::Gated(_) => (VarSet::new(), None),
            };
            if gives.is_disjoint(&need) {
                continue;
            }
            ks[i] = true;
            needed[i] = true;
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
        drop_other_documents(v, r, &key, pre, &needed, &mut ks);
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
        // Keyed by T's repr of (head, body): the first of equal magic rules stays.
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
    let about = cx.about;
    let settle = |cx: &mut Ctx<'v, '_>, keep: &[Vec<bool>]| -> Result<((crate::code_rule_dsl::logic::front::Components, Edges), Trial<'v>), (Error, Vec<Made<'v>>)> {
        let made = build(keep).map_err(|e| (e, Vec::new()))?;
        match strata(v, about, &made) {
            Ok(s) => Ok((s, Trial { made, ..Trial::default() })),
            Err(e) if e.unstratified().is_some() && gate => {
                let trial = gate_negations(cx, made, &full, &coarse, &decls).map_err(|e| (e, Vec::new()))?;
                match strata(v, about, &trial.made) {
                    Ok(s) => Ok((s, trial)),
                    Err(e) => Err((e, trial.made)),
                }
            }
            Err(e) => Err((e, made)),
        }
    };
    let settled = match settle(cx, &keep) {
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
    // Each trial keeps one more literal of one magic rule, which adds edges from that rule's head. The last whole
    // check's components, dependencies first, tell which edges leave them as they were: one into an earlier component
    // (nothing there reaches the head), or a positive one inside a component no negation closes. A literal whose new
    // edges are all such keeps the program stratified, as that check found it, and is kept without building and
    // stratifying the program again. Any other literal is checked whole, as before (and every one is with gates,
    // which a positive edge can unsettle). `RERAIL_MAGIC_CHECK=1` checks every one whole too.
    let places = |settled: &(crate::code_rule_dsl::logic::front::Components, Edges)| -> (BTreeMap<String, usize>, BTreeSet<usize>) {
        let (sccs, edges) = settled;
        let comp: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.to_owned(), i))).collect();
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
            let unchanged = !gate
                && comp.get(mh.0.as_str()).is_some_and(|&head| match (&before, &after) {
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
            match settle(cx, &keep) {
                Ok((settled, _)) => (comp, negated) = places(&settled),
                Err((e, _)) if e.unstratified().is_some() => {
                    assert!(!unchanged, "magic: a literal kept without a whole check unstratifies the program: {e}");
                    keep[i][j] = false
                }
                Err((e, _)) => return Err(Attempt::Failed(e)),
            }
        }
    }
    let trial = match settle(cx, &keep) {
        Ok((_, trial)) => trial,
        Err((e, _)) => return Err(Attempt::Failed(e)),
    };
    let extras_kept = magic.iter().zip(&keep).map(|((_, _, pre), ks)| pre.iter().zip(ks).filter(|((_, e), k)| *e && **k).count()).sum();
    let report = Report {
        adornments: seen.iter().filter(|(_, a)| a.contains('b')).map(|(r, a)| format!("{r}_{a}")).collect::<BTreeSet<_>>().into_iter().collect(),
        magic_rules: magic.len(),
        full: full.iter().filter(|r| cx.idb.contains(r.as_str())).cloned().collect(),
        extras_kept,
        gates: trial.gates.len(),
        full_for_negation: Vec::new(),
        reasons: Vec::new(),
    };
    Ok((trial, decls, report))
}

/// The relation rule `m` makes.
fn head_of<'a>(v: &'a Version, m: &'a Made<'_>) -> &'a str {
    m.head.as_ref().map_or_else(|| v.rel_name(m.rule.head().rel()), |(rel, _)| rel.as_str())
}

/// Gates the negations that demand puts in their own component (T's `gate_negations`): `!N(t)` becomes `done_N(t), !N(t)`, and the native engine copies `magic_N` into `done_N` only in
/// a wave after which nothing N reads changed. Within each such component the edges into guards are dropped: what is
/// left must be stratified, and its strata are the gates' levels. `decls`: the guards' declarations, which a gate
/// copies.
fn gate_negations<'v>(
    cx: &mut Ctx<'v, '_>,
    made: Vec<Made<'v>>,
    full: &BTreeSet<String>,
    coarse: &BTreeMap<(String, String), String>,
    decls: &[(String, Vec<(String, String)>)],
) -> Result<Trial<'v>, Error> {
    let v = cx.v;
    let Some(any) = made.first().map(|m| m.rule) else { return Ok(Trial { made, ..Trial::default() }) };
    let derived: BTreeSet<String> = made.iter().map(|m| head_of(v, m).to_owned()).collect();
    let mut edges: Edges = BTreeMap::new();
    for m in &made {
        for l in &m.body {
            if let Some((rel, neg, agg)) = l.atom(v)
                && derived.contains(rel)
            {
                edges.entry(head_of(v, m).to_owned()).or_default().insert((rel.to_owned(), neg || agg));
            }
        }
    }
    let sccs = tarjan(&derived, &edges);
    let comp: BTreeMap<String, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.clone(), i))).collect();
    let bad: BTreeSet<usize> = derived
        .iter()
        .flat_map(|x| edges.get(x).into_iter().flatten().filter(|(w, neg)| *neg && comp[w] == comp[x]).map(|_| comp[x]))
        .collect();
    let guard = |x: &str, w: &str| w.starts_with("magic_") && !x.starts_with("magic_");
    let (mut red, mut level) = (Edges::new(), BTreeMap::<String, usize>::new());
    for &i in &bad {
        let scc = &sccs[i];
        let e: Edges = scc
            .iter()
            .map(|x| {
                let inner = edges.get(x).into_iter().flatten().filter(|(w, _)| scc.contains(w) && !guard(x, w)).cloned().collect();
                (x.clone(), inner)
            })
            .collect();
        for (k, s) in tarjan(scc, &e).iter().enumerate() {
            for x in s {
                level.insert(x.clone(), k);
            }
        }
        for x in scc {
            for (w, n) in &e[x] {
                if *n && level[w] >= level[x] {
                    return Err(Error::NotStratifiable {
                        message: format!("not stratifiable: {x} depends negatively on {w} without its demand"),
                        component: scc.clone(),
                        negated: w.clone(),
                    });
                }
            }
        }
        red.extend(e);
    }
    let (mut rules, mut gates) = (Vec::new(), Vec::<(String, Gate)>::new());
    for m in made {
        let head = head_of(v, &m).to_owned();
        let mut body = Vec::new();
        for l in m.body {
            let inside = l.atom(v).is_some_and(|(rel, _, _)| comp.get(rel).is_some_and(|c| Some(c) == comp.get(head.as_str()) && bad.contains(c)));
            let (agg, neg) = (l.is_agg(), l.atom(v).is_some_and(|(_, neg, _)| neg));
            let x = match l {
                M::Own(x) if inside && (agg || neg) => x,
                l => {
                    body.push(l);
                    continue;
                }
            };
            let a = match x.view() {
                Lit::Atom(a) => a,
                Lit::Agg { atom, .. } => atom,
                _ => unreachable!("an atom"),
            };
            let rel = v.rel_name(a.rel());
            // An aggregated relation is full; a negated atom's adornment is the one it was demanded with, every
            // variable bound.
            let ad = if agg {
                "f".repeat(a.args().count())
            } else {
                let ad = if full.contains(rel) { "f".repeat(cx.arity(rel)) } else { cx.adorn(a, &atom_vars(a), full) };
                coarse.get(&(rel.to_owned(), ad.clone())).cloned().unwrap_or(ad)
            };
            let mut src = Some(format!("magic_{rel}_{ad}"));
            let g = if ad.contains('b') && comp.get(src.as_deref().unwrap_or_default()) == comp.get(rel) {
                (format!("done_{rel}_{ad}"), a.args().zip(ad.bytes()).filter(|(_, b)| *b == b'b').map(|(t, _)| Tm::Own(t)).collect())
            } else {
                src = None;
                (format!("done_{rel}"), vec![Tm::Zero])
            };
            if !gates.iter().any(|(name, _)| *name == g.0) {
                gates.push((g.0.clone(), Gate { src, rel: rel.to_owned(), level: level[rel], quiet: Vec::new(), deps: Vec::new() }));
            }
            body.push(M::Guard(g.0, g.1));
            body.push(M::Gated(x));
        }
        rules.push(Made { rule: m.rule, head: m.head, body });
    }
    let names: BTreeSet<String> = gates.iter().map(|(n, _)| n.clone()).collect();
    let mut done = Vec::new();
    for gate in &mut gates {
        let (name, g) = gate.clone();
        // The gate's own rule: `done_N_ad` copies `magic_N_ad`, and `done_N(0)` holds once `N` has a row.
        if let Some(src) = &g.src {
            let cols = decls.iter().find(|(n, _)| n == src).map(|(_, c)| c.clone()).ok_or_else(|| Error::Syntax(format!("{src} is not declared")))?;
            let vs: Vec<Tm<'_>> = (0..cols.len() as u32).map(Tm::Gate).collect();
            done.push((name.clone(), cols));
            rules.push(Made { rule: any, head: Some((name.clone(), vs.clone())), body: vec![M::Guard(src.clone(), vs)] });
        } else {
            done.push((name.clone(), vec![("k".to_owned(), "int".to_owned())]));
            let wild = vec![Tm::Wild; cx.arity(&g.rel)];
            rules.push(Made { rule: any, head: Some((name.clone(), vec![Tm::Zero])), body: vec![M::Guard(g.rel.clone(), wild)] });
        }
        let (mut reach, mut todo) = (BTreeSet::new(), vec![g.rel.clone()]);
        while let Some(x) = todo.pop() {
            if reach.insert(x.clone()) {
                todo.extend(red.get(&x).into_iter().flatten().map(|(w, _)| w.clone()));
            }
        }
        let scc = &sccs[comp[&g.rel]];
        let magic_of = |m: &str| m.strip_prefix("magic_").and_then(|m| m.rsplit_once('_')).map(|(r, _)| r.to_owned());
        let mut quiet: BTreeSet<String> = reach.clone();
        quiet.extend(scc.iter().filter(|m| magic_of(m).is_some_and(|r| reach.contains(&r))).cloned());
        // The other gates the relations it reads wait for (any atom of their rules, negated or not).
        let deps: BTreeSet<String> = rules
            .iter()
            .filter(|m| reach.contains(head_of(v, m)))
            .flat_map(|m| &m.body)
            .filter_map(|l| match l {
                M::Own(x) | M::Gated(x) => match x.view() {
                    Lit::Atom(a) => Some(v.rel_name(a.rel())),
                    _ => None,
                },
                M::Guard(rel, _) => Some(rel.as_str()),
                M::True => Some("magic_true"),
                M::Call(..) => None,
            })
            .filter(|rel| names.contains(*rel) && **rel != name)
            .map(str::to_owned)
            .collect();
        gate.1.quiet = quiet.into_iter().collect();
        gate.1.deps = deps.into_iter().collect();
    }
    Ok(Trial { made: rules, gates, done })
}

/// An atom written gated (its negation or aggregate kept): `M::Gated`.
struct Gating<'a>(&'a Version);

impl Rewrite for Gating<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        Some((self.0.rel_name(a.rel()).to_owned(), a.neg(), true))
    }
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
    // A gate's own rule's variables, `_g0` up to the last its head names.
    let gates = m.head.as_ref().map_or(0, |(_, args)| args.iter().filter(|t| matches!(t, Tm::Gate(_))).count());
    let gate_names: Vec<String> = (0..gates).map(|n| format!("_g{n}")).collect();
    let tm = |t: Tm<'v>| -> T<'_> {
        match t {
            Tm::Own(t) => t_of(r, t),
            Tm::Fresh(n) => T::Var(name_of(n)),
            Tm::Gate(n) => T::Var(gate_names.get(n as usize).map_or("_g", String::as_str)),
            Tm::Wild => T::Wild,
            Tm::Zero => T::Int(0),
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
            M::Gated(x) => w.lit_with(v, r, *x, &mut Gating(v)),
            M::Guard(rel, args) => {
                let a: Vec<T<'_>> = args.iter().map(|t| tm(*t)).collect();
                w.atom(rel.as_str(), &a)
            }
            M::Call(n, call) => w.cmp(T::Var(name_of(*n)), CmpOp::Eq, t_of(r, *call)),
            M::True => w.atom("magic_true", &[T::Int(0)]),
        };
    }
    w.finish()?;
    Ok(())
}
