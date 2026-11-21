//! A rule as the planner reads it (decision 0173, t36 item 2): the rule's handles in its version, its literals by
//! position, and each literal's variables as a set of slots. The planner plans a rule, not a (head, body) pair, and
//! its steps name the rule's literals by position (`Step::at`), as every other reader of a plan does.
//!
//! Each reading here was the tree's (`plan::ready`, `plan::is_join`, the digests and the text), stated over handles:
//! a plan searched over a `Body` is the plan the tree's search made, and its keys (`Body::digest`,
//! `Body::observed_key`) are the tree's keys (decision 0276).
use super::super::program_image::{AggRef, AtomRef, Builder, CmpOp, Lit, LitRef, RelId, RuleId, RuleRef, Slot, TermRef, Version};
use super::{Constant, Error, Result};
use std::cell::OnceCell;
use std::sync::Arc;

/// A set of a rule's variables, by slot.
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct VarSet(Vec<u64>);

impl VarSet {
    pub fn new() -> VarSet {
        VarSet(Vec::new())
    }
    pub fn of(slots: impl IntoIterator<Item = Slot>) -> VarSet {
        let mut s = VarSet::new();
        slots.into_iter().for_each(|x| {
            s.insert(x);
        });
        s
    }
    /// Adds `s`: whether it was not in the set.
    pub fn insert(&mut self, s: Slot) -> bool {
        let (w, b) = (s.index() / 64, s.index() % 64);
        if self.0.len() <= w {
            self.0.resize(w + 1, 0);
        }
        let was = self.0[w] >> b & 1 == 1;
        self.0[w] |= 1 << b;
        !was
    }
    pub fn remove(&mut self, s: Slot) {
        if let Some(w) = self.0.get_mut(s.index() / 64) {
            *w &= !(1 << (s.index() % 64));
        }
    }
    pub fn contains(&self, s: Slot) -> bool {
        self.0.get(s.index() / 64).is_some_and(|w| w >> (s.index() % 64) & 1 == 1)
    }
    pub fn extend(&mut self, o: &VarSet) {
        if self.0.len() < o.0.len() {
            self.0.resize(o.0.len(), 0);
        }
        self.0.iter_mut().zip(&o.0).for_each(|(a, b)| *a |= b);
    }
    pub fn is_subset(&self, o: &VarSet) -> bool {
        self.0.iter().enumerate().all(|(i, w)| w & !o.0.get(i).copied().unwrap_or(0) == 0)
    }
    pub fn is_disjoint(&self, o: &VarSet) -> bool {
        self.0.iter().zip(&o.0).all(|(a, b)| a & b == 0)
    }
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }
    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }
    /// The slots, in order.
    pub fn iter(&self) -> impl Iterator<Item = Slot> + '_ {
        self.0.iter().enumerate().flat_map(|(i, &w)| (0..64).filter(move |b| w >> b & 1 == 1).map(move |b| Slot((i * 64 + b) as u32)))
    }
    /// The slots of `self` not in `o`.
    pub fn minus(&self, o: &VarSet) -> VarSet {
        VarSet(self.0.iter().enumerate().map(|(i, w)| w & !o.0.get(i).copied().unwrap_or(0)).collect())
    }
}

/// The variables of term `t`, into `out`.
pub fn term_vars(t: TermRef<'_>, out: &mut VarSet) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

/// The variables of atom `a`.
pub fn atom_vars(a: AtomRef<'_>) -> VarSet {
    let mut out = VarSet::new();
    a.args().for_each(|t| term_vars(t, &mut out));
    out
}

/// The variables of literal `l`.
pub fn lit_vars(l: LitRef<'_>) -> VarSet {
    match l.view() {
        Lit::Atom(a) => atom_vars(a),
        Lit::Cmp { l, r, .. } => {
            let mut out = VarSet::new();
            term_vars(l, &mut out);
            term_vars(r, &mut out);
            out
        }
        Lit::Agg { var, over, atom, .. } => {
            let mut out = atom_vars(atom);
            out.insert(var);
            if let Some(o) = over {
                out.insert(o);
            }
            out
        }
        Lit::Nearest { n, a, .. } => VarSet::of([n, a]),
        Lit::Ast { document, node, captures, .. } => VarSet::of([document, node].into_iter().chain(captures.map(|(_, v)| v))),
    }
}

/// A rule of a version, as the planner reads it.
pub struct Body<'v> {
    pub v: &'v Version,
    pub rule: RuleRef<'v>,
    lits: Vec<LitRef<'v>>,
    vars: OnceCell<Vec<VarSet>>,
    args: OnceCell<Vec<Vec<TermRef<'v>>>>,
    /// The rule's digest (`key`), made at the first ask: a plan call reads it for each of its keys.
    key: OnceCell<u128>,
    /// The version the rule's source (`RuleRef::source`) is a rule of: the run's version 0, for a rule of a
    /// maintained form (`Versions::at`); `None` for every other rule.
    sources: Option<&'v Version>,
    /// What is bound before its first step: its head's plain variables (`with_head_bound`), a recheck's, run once
    /// per candidate row (decision 0364 stage 3, Q208); or given variables (`with_bound`), a maintain line's led by a
    /// delta that binds them (a negated changed read's, an aggregate's group key: 0364 R4). `None`: nothing.
    pre: Option<Pre>,
}

/// What a body has bound before its first step (`Body::pre`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pre {
    Head,
    Vars(VarSet),
}

impl<'v> Body<'v> {
    pub fn of(v: &'v Version, rule: RuleRef<'v>) -> Body<'v> {
        Body { v, rule, lits: rule.body().collect(), vars: OnceCell::new(), args: OnceCell::new(), key: OnceCell::new(), sources: None, pre: None }
    }
    /// The rule with its head's plain variables bound before its first step: a recheck of one candidate row
    /// (decision 0364 stage 3). The planner prices and keeps its order apart from the rule's own (`key` names it):
    /// each step starts from those variables bound and one row (Q208).
    pub fn with_head_bound(&self) -> Body<'v> {
        self.with_pre(Pre::Head)
    }
    /// The rule with `vars` bound before its first step, by one row: a maintain line led by a delta whose rows bind
    /// them, the rest of the body planned from there (0364 R4). Kept apart from the rule's own order, as a recheck's.
    pub fn with_bound(&self, vars: VarSet) -> Body<'v> {
        self.with_pre(Pre::Vars(vars))
    }
    fn with_pre(&self, pre: Pre) -> Body<'v> {
        Body { v: self.v, rule: self.rule, lits: self.lits.clone(), vars: OnceCell::new(), args: OnceCell::new(), key: OnceCell::new(), sources: self.sources, pre: Some(pre) }
    }
    /// Whether its head's plain variables are bound before its first step (a recheck, `with_head_bound`).
    pub fn head_bound(&self) -> bool {
        matches!(self.pre, Some(Pre::Head))
    }
    /// The variables bound before its first step: its head's plain ones for a recheck (`with_head_bound`), the given
    /// ones (`with_bound`), else none. A computed term's variables are not bound by the candidate.
    pub fn bound_before(&self) -> VarSet {
        match &self.pre {
            None => VarSet::new(),
            Some(Pre::Head) => VarSet::of(self.head().args().filter_map(|t| t.var())),
            Some(Pre::Vars(v)) => v.clone(),
        }
    }
    /// The rule this one was written from (decision 0258): for a maintenance rule of a session's form (`h__del`,
    /// `h__ins`, `h__redo`, …), its source rule `h :- …` of the run's version. `None` for a rule not written from one.
    pub fn origin(&self) -> Option<Body<'v>> {
        let v0 = self.sources?;
        let src = Body::of(v0, v0.rule(self.rule.source()?)?);
        // A maintenance rule derives a relation of its source's (`h__del` of `h`): any other is not its source.
        let (head, of) = (self.rel(self.head()), src.rel(src.head()));
        (head.len() > of.len() + 2 && head.starts_with(of) && head[of.len()..].starts_with("__")).then_some(src)
    }
    /// Each literal's literal of `src`, the rule it was written from (`origin`), where it is one: the same atom over
    /// the same terms, of the same relation or one of its maintenance relations (`r__old`, `r__minus`, `r__del`, …),
    /// or the same literal of another kind. The rewrite copies a source literal, or writes one of its maintenance
    /// relations over its terms (`maintain::write_ir`); what it adds (`h__old(..)`, `h__del(..)`, a group) is no
    /// source literal's. Each source literal is taken once, one of the same relation before one of its maintenance
    /// relations.
    pub fn source_map(&self, src: &Body<'_>) -> Vec<Option<usize>> {
        let derived = |r: &str, of: &str| r.len() > of.len() + 2 && r.starts_with(of) && r[of.len()..].starts_with("__");
        let digest = |b: &Body<'_>, i: usize| super::super::derived::digest(|d| b.digest_lit(d, b.lits[i]));
        let mut used = vec![false; src.len()];
        let mut map = vec![None; self.len()];
        for exact in [true, false] {
            for (i, m) in map.iter_mut().enumerate() {
                if m.is_some() {
                    continue;
                }
                let found = (0..src.len()).find(|&j| {
                    !used[j]
                        && match (self.atom(i), src.atom(j)) {
                            (Some(a), Some(c)) => {
                                let (r, of) = (self.rel(a), src.rel(c));
                                (if exact { r == of } else { derived(r, of) })
                                    && self.plain_atom(i).is_some() == src.plain_atom(j).is_some()
                                    && self.args(i).len() == src.args(j).len()
                                    && self.args(i).iter().zip(src.args(j)).all(|(&t, &u)| self.same_term(t, src, u))
                            }
                            (None, None) => exact && digest(self, i) == digest(src, j),
                            _ => false,
                        }
                });
                if let Some(j) = found {
                    used[j] = true;
                    *m = Some(j);
                }
            }
        }
        map
    }
    /// Whether term `t` of this body is term `u` of `o`: the same text, variables by their names.
    fn same_term(&self, t: TermRef<'_>, o: &Body<'_>, u: TermRef<'_>) -> bool {
        match (t, u) {
            (TermRef::Var(a), TermRef::Var(b)) => self.var(a) == o.var(b),
            (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
            (TermRef::Int(a), TermRef::Int(b)) => a == b,
            (TermRef::Branded(a), TermRef::Branded(b)) => a == b,
            (TermRef::Str(a), TermRef::Str(b)) => self.v.text(a) == o.v.text(b),
            (TermRef::Call(f, xs), TermRef::Call(g, ys)) => {
                self.v.text(f) == o.v.text(g) && xs.count() == ys.count() && xs.zip(ys).all(|(x, y)| self.same_term(x, o, y))
            }
            _ => false,
        }
    }
    /// The arguments of literal `i`'s atom (its own, or its aggregate's; none for another literal).
    pub fn args(&self, i: usize) -> &[TermRef<'v>] {
        &self.args.get_or_init(|| self.lits.iter().map(|l| l.atom().map_or_else(Vec::new, |a| a.args().collect())).collect())[i]
    }
    /// Whether another kept read already proves this positive read, with the same bound columns.
    pub(super) fn implied_read(&self, at: usize, kept: &[usize]) -> bool {
        let Some(read) = self.plain_atom(at).filter(|a| !a.neg() && !a.gated()) else { return false };
        let args = self.args(at);
        kept.iter().copied().filter(|&other| other != at).any(|other| {
            if !self.plain_atom(other).is_some_and(|a| !a.neg() && !a.gated() && a.rel() == read.rel()) {
                return false;
            }
            let terms = self.args(other);
            args.len() == terms.len()
                && args.iter().zip(terms).all(|(&a, &b)| matches!(a, TermRef::Wild) || self.same_term(a, self, b))
                && (other < at || args.iter().zip(terms).any(|(&a, &b)| matches!(a, TermRef::Wild) && !matches!(b, TermRef::Wild)))
        })
    }
    /// The head as text, written only when shown (`{head}` in a message).
    pub fn head_text(&self) -> HeadText<'_, 'v> {
        HeadText(self)
    }
    /// The literals read by `steps`, as text (`plan::shown`).
    pub fn shown(&self, steps: &[super::Step]) -> Vec<String> {
        steps.iter().map(|s| self.show(s.at)).collect()
    }
    /// How many literals read a relation: atoms and aggregates.
    pub fn read_count(&self) -> usize {
        self.lits.iter().filter(|l| l.atom().is_some()).count()
    }
    /// The demand that drives its rule whatever the statistics say: a demand-restricted aggregate's
    /// (`rr_demand_agg_…`, a positive atom), whose witnesses are enumerated from its requested keys, or the tuple
    /// demand that bounds a Cartesian product (`product_demand::PREFIX`): the product's only bound, so it leads; or a
    /// deferred join's requests and a pair cache's guard, the same. Not a demanded relation's own demand (decision
    /// 0048's `demand_x`): an atom a constant restricts may lead it (`demand_bounds_lead`, Q59).
    pub fn demand_first(&self) -> Option<usize> {
        let head = self.rel(self.head());
        (0..self.len()).find(|&i| self.plain_atom(i).is_some_and(|a| {
            let rel = self.rel(a);
            // A pair cache's pairs are made only past its guard (`rr_pair_cache_…_small`), and a join the deferred
            // joins pass bounds by its readers' requests (`product_demand::DEFERRED_PAIRS`) starts from them.
            let cache = |base: &str| head.strip_prefix("rr_pair_cache_").and_then(|s| s.strip_suffix("_pairs")) == Some(base);
            !a.neg()
                && (rel.contains("rr_demand_agg_")
                    || rel.strip_prefix("rr_pair_cache_").and_then(|s| s.strip_suffix("_small")).is_some_and(cache)
                    || [super::corpus::product_demand::PREFIX, super::corpus::product_demand::DEFERRED_PAIRS].iter().any(|&p| rel.strip_prefix(p) == Some(head)))
        }))
    }
    /// A demanded relation's own demand (decision 0048's `x :- demand_x, …`), which bounds what its rule reads.
    pub fn own_demand(&self) -> Option<usize> {
        let head = self.rel(self.head());
        (0..self.len()).find(|&i| self.plain_atom(i).is_some_and(|a| !a.neg() && self.rel(a).strip_prefix("demand_").is_some_and(|r| r == head)))
    }
    /// Whether an order led by literal `lead` keeps a demanded relation's rule to what its demand bounds (Q59): led
    /// by the demand, or by an atom a constant restricts (a column a constant names, or a variable an equality binds
    /// to one: `kind($i, #k1139)` reads one kind's nodes). An unrestricted atom first (`col(v, c)`) scans the corpus
    /// per chunk: 29 M rows, not 34 k, on PostHog. Forcing the demand first always cost 66x on a restricted lead
    /// (`not_name` on 50 files of B-now: a 42 k-row demand against a kind of 498 nodes).
    pub fn demand_bounds_lead(&self, lead: usize) -> bool {
        let Some(d) = self.own_demand() else { return true };
        if lead == d {
            return true;
        }
        let Some(a) = self.plain_atom(lead) else { return false };
        if a.neg() {
            return false;
        }
        let bound: Vec<_> = (0..self.len())
            .filter_map(|l| match self.view(l) {
                Lit::Cmp { l: x, op: CmpOp::Eq, r: y } => match (x, y) {
                    (TermRef::Var(v), t) | (t, TermRef::Var(v)) if self.constant(t).is_some() => Some(v),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        self.args(lead).iter().any(|&t| self.constant(t).is_some() || t.var().is_some_and(|v| bound.contains(&v)))
    }
    /// The share of rows filter `i` passes on (decision 0297): measured by kind of filter, rows weighted, over the
    /// 300-file bench's 317 rules (`profile`'s `native.tsv`, each step's rows in and out). An equality that binds a
    /// variable no join of the body binds passes every row; one that tests passes as a test of its kind does.
    pub fn filter_pass(&self, i: usize) -> f64 {
        match self.view(i) {
            Lit::Atom(a) if a.neg() => 0.95,
            Lit::Cmp { l, op, r } => {
                let joined = |t: TermRef<'_>| t.var().is_some_and(|v| (0..self.len()).any(|j| self.is_join(j) && self.vars(j).contains(v)));
                let free = |t: TermRef<'_>| t.var().is_some() && !joined(t);
                let call = matches!(l, TermRef::Call(..)) || matches!(r, TermRef::Call(..));
                // A kind, field or name is branded (decision 0270): a constant like any other.
                let constant = |t: TermRef<'_>| matches!(t, TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_));
                match op {
                    CmpOp::Eq if free(l) || free(r) => 1.0,
                    CmpOp::Eq if call => 0.4,
                    CmpOp::Eq if constant(l) || constant(r) => 0.6,
                    CmpOp::Eq => 0.9,
                    CmpOp::Ne => 0.97,
                    // A bound of an id's range (`$n < 6917529027641081856`, `$d < 1610612736`), as lowering writes
                    // it: every row of the right kind of id passes.
                    _ if [l, r].iter().any(|t| matches!(t, TermRef::Int(c) if c.unsigned_abs() >= 1 << 30)) => 0.98,
                    _ if constant(l) || constant(r) => 0.7,
                    _ => 1.0 / 3.0,
                }
            }
            _ => 1.0,
        }
    }
    /// Whether literals `i` and `j` read equal atoms: the same relation, flags and arguments.
    pub fn same_atom(&self, i: usize, j: usize) -> bool {
        match (self.atom(i), self.atom(j)) {
            (Some(a), Some(b)) => a.gated() == b.gated() && self.show_atom(a) == self.show_atom(b),
            _ => false,
        }
    }
    /// The variable named `name`.
    pub fn slot(&self, name: &str) -> Option<Slot> {
        self.rule.vars().position(|a| self.v.text(a) == name).map(|k| Slot(k as u32))
    }
    /// The relations the planner reads statistics of (`Model::read_by`): none for a body of one positive atom of
    /// distinct variables, which is planned without them.
    pub fn reads_statistics(&self) -> Vec<&'v str> {
        let pos: Vec<usize> = (0..self.len()).filter(|&i| self.atom(i).is_some_and(|a| !a.neg())).collect();
        let plain = pos.len() == 1 && {
            let args = self.args(pos[0]);
            let vars: Vec<Slot> = args.iter().filter_map(|t| t.var()).collect();
            vars.len() == args.len() && VarSet::of(vars.iter().copied()).len() == vars.len()
        };
        if plain {
            return Vec::new();
        }
        (0..self.len()).filter_map(|i| self.atom(i)).map(|a| self.rel(a)).collect()
    }
    /// The signature of literal `i`'s atom with `bound` bound, under which its fanouts are observed (T's `atom_sig`).
    pub fn sig(&self, i: usize, bound: &VarSet) -> super::Sig {
        let cols = self.args(i).iter().enumerate().filter_map(|(c, &t)| match t {
            TermRef::Var(v) if bound.contains(v) => Some((c, None)),
            _ => self.constant(t).map(|k| (c, Some(k))),
        }).collect();
        (self.rel(self.atom(i).expect("an atom")).to_owned(), cols)
    }
    pub fn len(&self) -> usize {
        self.lits.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lits.is_empty()
    }
    pub fn head(&self) -> AtomRef<'v> {
        self.rule.head()
    }
    pub fn lit(&self, i: usize) -> LitRef<'v> {
        self.lits[i]
    }
    pub fn view(&self, i: usize) -> Lit<'v> {
        self.lits[i].view()
    }
    /// The atom literal `i` reads: its own, or its aggregate's.
    pub fn atom(&self, i: usize) -> Option<AtomRef<'v>> {
        self.lits[i].atom()
    }
    /// Literal `i` as an atom (positive or negated), not an aggregate's.
    pub fn plain_atom(&self, i: usize) -> Option<AtomRef<'v>> {
        match self.lits[i].view() {
            Lit::Atom(a) => Some(a),
            _ => None,
        }
    }
    /// Literal `i`'s variables.
    pub fn vars(&self, i: usize) -> &VarSet {
        &self.vars.get_or_init(|| self.lits.iter().map(|&l| lit_vars(l)).collect())[i]
    }
    /// The relation's name.
    pub fn rel(&self, a: AtomRef<'_>) -> &'v str {
        self.v.rel_name(a.rel())
    }
    /// The name of variable `s`.
    pub fn var(&self, s: Slot) -> &'v str {
        self.v.text(self.rule.var(s))
    }
    /// The position of the literal whose atom is `a`, its own or its aggregate's.
    pub fn position_of(&self, a: AtomRef<'_>) -> Option<usize> {
        self.lits.iter().position(|l| l.atom().is_some_and(|b| b.node() == a.node()))
    }
    /// A constant term's value.
    pub fn constant(&self, t: TermRef<'_>) -> Option<Constant> {
        match t {
            TermRef::Int(i) => Some(Constant::Int(i)),
            TermRef::Branded(a) => Some(Constant::Int(a.value())),
            TermRef::Str(a) => Some(Constant::Str(self.v.text(a).to_owned())),
            _ => None,
        }
    }
    /// Whether literal `i` joins: a positive atom, or `nearest`.
    pub fn is_join(&self, i: usize) -> bool {
        match self.view(i) {
            Lit::Atom(a) => !a.neg(),
            Lit::Nearest { .. } => true,
            _ => false,
        }
    }
    /// Whether filter `i` can run with `bound` bound: `$x = e` assigns $x if it is unbound when `e`'s variables are
    /// bound (SQL and the native `cat`/`eq` steps bind an unbound left side); otherwise it is a test. An equality of a
    /// bound variable or constant with an unbound variable, `$s = $a`, assigns the right side as `$a = $s` would
    /// (`binds_right`).
    pub fn ready(&self, i: usize, bound: &VarSet) -> bool {
        match self.view(i) {
            Lit::Atom(a) if a.neg() => self.vars(i).is_subset(bound),
            Lit::Cmp { l, op, r } => self.cmp_binds(l, op, r, bound).is_some() || binds_right(l, op, r, bound).is_some(),
            Lit::Agg { var, over, atom, op } => {
                let mut need = atom_vars(atom);
                need.remove(var);
                if let Some(o) = over {
                    need.remove(o);
                }
                if let AggRef::Join { by, .. } = op {
                    by.for_each(|b| need.remove(b));
                }
                need.is_subset(bound)
            }
            Lit::Nearest { n, a, .. } => bound.contains(n) || bound.contains(a),
            _ => true,
        }
    }
    /// A comparison's variables once it runs, or `None` if it cannot run yet (`front::cmp_binds`).
    pub fn cmp_binds(&self, l: TermRef<'_>, op: CmpOp, r: TermRef<'_>, bound: &VarSet) -> Option<VarSet> {
        let mut rv = VarSet::new();
        term_vars(r, &mut rv);
        if op == CmpOp::Eq
            && let TermRef::Var(x) = l
            && !bound.contains(x)
        {
            return rv.is_subset(bound).then(|| VarSet::of([x]));
        }
        let mut all = rv.clone();
        term_vars(l, &mut all);
        if all.is_subset(bound) {
            return Some(VarSet::new());
        }
        if op == CmpOp::Eq && matches!(l, TermRef::Var(_)) && self.is_cat(r) {
            return Some(rv.minus(bound));
        }
        None
    }
    /// `@cat` over variables only.
    pub fn is_cat(&self, t: TermRef<'_>) -> bool {
        matches!(t, TermRef::Call(f, args) if self.v.text(f) == "cat" && { args.clone().all(|a| matches!(a, TermRef::Var(_))) })
    }

    /// The rule written into a derived value's inputs (`logic::derived`), field by field: its head and body, the same
    /// pieces in the same order as its text, without making the text.
    pub(in crate::code_rule_dsl) fn digest(&self, d: &mut super::super::derived::Inputs) {
        self.digest_pre(d);
        self.digest_atom(d, self.head());
        d.u64(self.lits.len() as u64);
        for &l in &self.lits {
            self.digest_lit(d, l);
        }
    }

    /// What is bound before its first step, written into a derived value's inputs: a recheck's order is not the
    /// rule's, kept apart (Q208); nor a line's planned from given variables bound (0364 R4).
    pub(in crate::code_rule_dsl) fn digest_pre(&self, d: &mut super::super::derived::Inputs) {
        match &self.pre {
            None => {}
            Some(Pre::Head) => d.str("head bound"),
            Some(Pre::Vars(v)) => {
                d.str("bound");
                d.u64(v.0.len() as u64);
                for w in &v.0 {
                    d.u64(*w);
                }
            }
        }
    }
    /// The rule's digest (`digest`), made once: what every key of the rule writes (`plan_key`, `line_key`,
    /// `observed_key`) in place of the rule's text.
    pub fn key(&self) -> u128 {
        *self.key.get_or_init(|| super::super::derived::digest(|d| self.digest(d)))
    }
    /// The key observations of this rule's steps are kept under (`Stats::obs`): its head and body, digested (`key`,
    /// not its text written out: it is asked at every search). A fanout through a bound column depends on which values
    /// the rule's bindings bring: `docof($x, $d)` with `$d` bound reads a whole document, and one rule's bindings fall
    /// in 11-node documents while another's fall in documents of thousands of nodes. Pooled over rules, the first set
    /// the second's estimate (`up_once`: 1,285 rows a binding, priced at 10).
    pub fn observed_key(&self) -> u64 {
        let d = self.key();
        (d as u64) ^ ((d >> 64) as u64)
    }
    fn digest_term(&self, d: &mut super::super::derived::Inputs, t: TermRef<'_>) {
        match t {
            TermRef::Var(s) => {
                d.u64(0);
                d.str(self.var(s));
            }
            TermRef::Wild => d.u64(1),
            TermRef::Int(i) => {
                d.u64(2);
                d.u64(i as u64);
            }
            TermRef::Branded(b) => {
                d.u64(6);
                d.u64(b.class as u64);
                d.u64(u64::from(b.atom));
            }
            TermRef::Str(a) => {
                d.u64(3);
                d.str(self.v.text(a));
            }
            TermRef::Top => d.u64(4),
            TermRef::Call(f, args) => {
                d.u64(5);
                d.str(self.v.text(f));
                d.u64(args.count() as u64);
                args.for_each(|a| self.digest_term(d, a));
            }
        }
    }
    pub(in crate::code_rule_dsl) fn digest_atom(&self, d: &mut super::super::derived::Inputs, a: AtomRef<'_>) {
        d.str(self.rel(a));
        d.u64(u64::from(a.neg()) | u64::from(a.gated()) << 1);
        d.u64(a.args().count() as u64);
        a.args().for_each(|t| self.digest_term(d, t));
    }
    pub(in crate::code_rule_dsl) fn digest_lit(&self, d: &mut super::super::derived::Inputs, l: LitRef<'_>) {
        match l.view() {
            Lit::Atom(a) => {
                d.u64(0);
                self.digest_atom(d, a);
            }
            Lit::Cmp { l, op, r } => {
                d.u64(1);
                d.str(op.as_str());
                self.digest_term(d, l);
                self.digest_term(d, r);
            }
            Lit::Agg { var, op, over, atom } => {
                d.u64(2);
                d.str(self.var(var));
                match op {
                    AggRef::Join { by, sep } => {
                        d.u64(4);
                        d.u64(by.count() as u64);
                        by.for_each(|b| d.str(self.var(b)));
                        d.str(self.v.text(sep));
                    }
                    AggRef::Count => d.u64(0),
                    AggRef::Min => d.u64(1),
                    AggRef::Max => d.u64(2),
                    AggRef::Sum => d.u64(3),
                    AggRef::Product => d.u64(5),
                }
                d.str(over.map_or("\u{0}", |o| self.var(o)));
                self.digest_atom(d, atom);
            }
            Lit::Nearest { n, a, kindset } => {
                d.u64(3);
                d.str(self.var(n));
                d.str(self.var(a));
                d.str(self.v.text(kindset));
            }
            Lit::Ast { document, node, matcher, captures } => {
                d.u64(4);
                d.str(self.var(document));
                d.str(self.var(node));
                d.str(self.v.text(matcher));
                d.u64(captures.count() as u64);
                for (k, v) in captures {
                    d.str(self.v.text(k));
                    d.str(self.var(v));
                }
            }
        }
    }

    /// Literal `i` as text, as `show_rule` writes it.
    pub fn show(&self, i: usize) -> String {
        self.v.show_lit(self.rule, self.lits[i])
    }
    /// The head as text.
    pub fn show_head(&self) -> String {
        self.v.show_atom(self.rule, self.head())
    }
    pub fn show_atom(&self, a: AtomRef<'_>) -> String {
        self.v.show_atom(self.rule, a)
    }
}

/// A rule's head as text (`Body::head_text`).
pub struct HeadText<'a, 'v>(&'a Body<'v>);

impl std::fmt::Display for HeadText<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.show_head())
    }
}

/// The variable `l = r` assigns from its right side: `r` an unbound variable and `l` a bound variable or a constant
/// (`$s = $a` with `$s` bound reads as `$a = $s`). Without it, `p($s), q($a), $s = $a` could only be ordered as a
/// cross product of `p` and `q` narrowed after by the test (decision 0099 item 4: `fl4317_typing_root`, 14 k rows,
/// then 2.86 M, then 603). `RERAIL_EQUALITY_BINDS=off` keeps the left side the only one assigned, for comparison.
pub(super) fn binds_right(l: TermRef<'_>, op: CmpOp, r: TermRef<'_>, bound: &VarSet) -> Option<Slot> {
    if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_EQUALITY_BINDS").is_ok_and(|v| v == "off")) || op != CmpOp::Eq {
        return None;
    }
    let TermRef::Var(y) = r else { return None };
    let ready = match l {
        TermRef::Var(x) => bound.contains(x),
        TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => true,
        _ => false,
    };
    (ready && !bound.contains(y)).then_some(y)
}

/// The versions of the programs whose rules a run plans, and a rule of one named by its key (`RuleKey`: the version's
/// place and the rule's place in the program's rules). The run's program is version 0 (`of_placed`), and a maintained
/// form a version of its own (`push`). A program is added once it is final: its rules do not move after.
#[derive(Clone, Default)]
pub struct Versions(Vec<(usize, usize, Placed)>);


/// A writer frozen to what a pass reads (decision 0276 §3): a `Placed`, its `Fields` read afresh at their first
/// reader. A writer cannot fail to make a version, so an error is an error, never a fall back to another form.
pub trait Freeze {
    /// The version the writer makes, its rules in its order.
    fn freeze(self) -> std::result::Result<Placed, String>;
    /// The version the writer makes, the program's rule `k` its rule `order[k]` as the writer numbered them, read
    /// back where the edit folded (`Builder::finish_placing`); `whole` where it holds the program's other fields too.
    fn freeze_placed(self, order: Vec<RuleId>, whole: bool) -> std::result::Result<Placed, String>;
}

impl Freeze for Builder<'_> {
    fn freeze(self) -> std::result::Result<Placed, String> {
        Ok(Placed::whole(self.finish()?.0))
    }
    fn freeze_placed(self, mut order: Vec<RuleId>, whole: bool) -> std::result::Result<Placed, String> {
        let (v, _, folded) = self.finish_placing()?;
        if let Some(folded) = folded {
            order.iter_mut().for_each(|id| *id = folded[id]);
        }
        Ok(Placed::new(v, order, whole))
    }
}

/// A program pass (decision 0276 §3), from the program `p` to what it makes of it, or `None` where it changes nothing:
/// `f`. Its profile span (`span`, the name the profile and the decisions cite) and its version trace (`site`,
/// `version_trace`, §6) are taken here, once for every pass, not by each pass.
pub fn pass<T, E>(site: &'static str, span: &'static str, p: &Placed, f: impl FnOnce(&Placed) -> std::result::Result<Option<(T, Placed)>, E>) -> std::result::Result<Option<(T, Placed)>, E> {
    let out = {
        let _span = super::super::profile::span(|| span.into());
        f(p)?
    };
    if let Some((_, q)) = &out
        && super::super::version_trace::on()
    {
        super::super::version_trace::note(site, &p.v, &q.v);
    }
    Ok(out)
}

/// A program's version, and where its rules are in it: the program's rule `k` is the version's rule `ids[k]` (rule
/// `k` where `ids` is `None`, a version made of the rules in order). A pass's overlay keeps its rules where its edits
/// put them (a reader's copies after the rules it does not touch), and the program, decoded, in the tree pass's order.
#[derive(Clone)]
pub struct Placed {
    pub v: Arc<Version>,
    ids: Option<Arc<[RuleId]>>,
    /// Whether `v` holds the program's other fields too (`Placed::whole`), not its rules alone.
    pub whole: bool,
    /// The version's other fields, read once at the first `Fields::of` and shared by every clone after it.
    pub(super) fields: std::sync::OnceLock<Arc<super::fields::FieldsData>>,
    /// The ids of `v`'s rules in order, made at the first `id` where `ids` is `None` and `v` numbers its rules other
    /// than `0..size` (a view or an overlay: a split's part, decision 0244).
    positions: std::sync::OnceLock<Arc<[RuleId]>>,
}

impl Placed {
    /// The run's version of the program whose image this is (t36 item 7, step 2): normalized for the SQL kernel
    /// (`passes::normalize`, a derivation), with its stated facts apart where `apart` (`passes::stated_apart`, an
    /// overlay), as `plan::run` normalizes the program. Also the relations given a presence column (the sentinels),
    /// and the pairs `(rel, rel__stated)`, by name.
    pub fn normalized(&self, apart: bool) -> Result<Normalized> {
        use super::super::program_image::passes;
        #[cfg(any(test, feature = "test-hooks"))]
        super::run_counts().normalized.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !self.whole || self.ids.is_some() {
            return Err(Error::Plan("the program has no image: its run reads it from its version".into()));
        }
        // Each held by the version it is of: made once however many runs (a corpus run's chunks) read it.
        use super::super::program_image::manager::analysis;
        let normal = analysis::<passes::NormalForm>(&self.v).map_err(Error::Plan)?;
        let sentinels = normal.sentinels.iter().map(|&r| normal.v.rel_name(r).to_owned()).collect();
        let (v, stated) = if apart {
            let a = analysis::<passes::ApartForm>(&normal.v).map_err(Error::Plan)?;
            (a.v.clone(), a.stated.clone())
        } else {
            (normal.v.clone(), Vec::new())
        };
        let stated = stated.iter().map(|&(r, s)| (v.rel_name(r).to_owned(), v.rel_name(s).to_owned())).collect();
        Ok(Normalized { placed: Placed { v, ids: None, whole: true, fields: Default::default(), positions: Default::default() }, sentinels, stated })
    }
    /// Its components, dependencies first, in the tree's order (`analyses::components_by_name`). Held by the version
    /// (decision 0281 §2): computed once however many readers and `Placed`s ask.
    pub fn components(&self) -> Result<Arc<super::super::program_image::analyses::Components>> {
        super::super::program_image::manager::analysis::<super::super::program_image::analyses::Components>(&self.v).map_err(Error::Plan)
    }
    /// A version whose rules are `v`'s in order (a program's image).
    pub fn whole(v: Arc<Version>) -> Placed {
        Placed { v, ids: None, whole: true, fields: Default::default(), positions: Default::default() }
    }
    /// `v`, the program's rule `k` its rule `ids[k]`.
    pub fn new(v: Arc<Version>, ids: Vec<RuleId>, whole: bool) -> Placed {
        Placed { v, ids: Some(ids.into()), whole, fields: Default::default(), positions: Default::default() }
    }
    /// The program's digest (decision 0173 §7): its version's, kept as the version was made, less the rules the
    /// program leaves out of it (where it is some of its version's rules). A sum, so it
    /// does not see the rules' order.
    pub fn digest(&self) -> u64 {
        
        match &self.ids {
            None => self.v.digest(),
            Some(ids) => {
                let all = self.v.rules().fold(0u64, |a, r| a.wrapping_add(r.hash()));
                ids.iter().fold(self.v.digest().wrapping_sub(all), |a, &id| a.wrapping_add(self.v.rule(id).map_or(0, |r| r.hash())))
            }
        }
    }
    /// How many rules the program has.
    pub fn len(&self) -> usize {
        self.ids.as_ref().map_or(self.v.size(), |ids| ids.len())
    }
    /// The id of the program's rule `k`.
    pub fn id(&self, k: usize) -> RuleId {
        match &self.ids {
            Some(ids) => ids[k],
            None if matches!(*self.v, Version::Base { .. }) => RuleId(k as u32),
            None => self.positions.get_or_init(|| self.v.rules().map(|r| r.id()).collect())[k],
        }
    }
    /// The program's rules' ids, in its order.
    pub fn order(&self) -> Vec<RuleId> {
        match &self.ids {
            Some(ids) => ids.to_vec(),
            None => self.v.rules().map(|r| r.id()).collect(),
        }
    }
    /// The program's rules, in its order: its version's (whose ids are ranges where it is a view, not `0..len`), or
    /// those `ids` names.
    pub fn rules(&self) -> Box<dyn Iterator<Item = RuleRef<'_>> + '_> {
        match &self.ids {
            None => self.v.rules(),
            Some(ids) => Box::new(ids.iter().map(|&id| self.v.rule(id).expect("a rule of its version"))),
        }
    }
    /// The program's rule `k`.
    pub fn rule(&self, k: usize) -> RuleRef<'_> {
        self.v.rule(self.id(k)).expect("a rule of its version")
    }
    /// The program's rule `k` as the planner reads it.
    pub fn body(&self, k: usize) -> Body<'_> {
        Body::of(&self.v, self.rule(k))
    }
}

/// A program's version as `plan::run` normalizes it (`Placed::normalized`): the version, the relations given a
/// presence column, and the pairs `(rel, rel__stated)` whose stated facts were moved apart.
pub struct Normalized {
    pub placed: Placed,
    pub sentinels: std::collections::BTreeSet<String>,
    pub stated: Vec<(String, String)>,
}

/// A rule of a run: the version it is a rule of (its place in the run's `Versions`; the program's is 0) and its id
/// there. What the run holds of a rule across waves and suspensions, in place of the rule.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct RuleKey {
    v: u32,
    id: u32,
}

impl RuleKey {
    /// Rule `k` of the program (version 0).
    pub fn program(k: usize) -> RuleKey {
        RuleKey { v: 0, id: k as u32 }
    }
    /// Rule `k` of the run's `v`-th version (its place in the program's rules the version was made from).
    pub fn new(v: usize, k: usize) -> RuleKey {
        RuleKey { v: v as u32, id: k as u32 }
    }
    /// The rule's number in the program, a rule of the program's version (a maintained form's has none).
    pub fn number(self) -> Option<usize> {
        (self.v == 0).then_some(self.id as usize)
    }
    /// The version the rule is a rule of (its place in the run's `Versions`).
    pub fn version(self) -> usize {
        self.v as usize
    }
    /// The rule's place among its version's program's rules.
    pub fn index(self) -> usize {
        self.id as usize
    }
}

impl Versions {
    /// `placed` alone, the run's version 0, for a caller that holds no tree (the composed path's edited versions): no
    /// rule or program is found by its address.
    pub fn of_placed(placed: Placed) -> Versions {
        Versions(vec![(0, 0, placed)])
    }
    /// Adds `placed` (a maintained form, a derivation of the run's version: decision 0241): its place.
    pub fn push(&mut self, placed: Placed) -> usize {
        self.0.push((0, 0, placed));
        self.0.len() - 1
    }
    /// The run's `k`-th version.
    pub fn placed(&self, k: usize) -> &Placed {
        &self.0[k].2
    }
    /// How many versions the run holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Whether the run holds no version.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// The keys of the `v`-th version's rules, `n` of them (its program's rule count), in the program's order.
    pub fn keys(v: usize, n: usize) -> impl Iterator<Item = RuleKey> {
        (0..n).map(move |k| RuleKey::new(v, k))
    }
    /// The fields of the run's `k`-th version (the program's is 0), read once: `None` where it is rules alone.
    pub fn fields(&self, k: usize) -> Option<super::fields::Fields<'_>> {
        self.0.get(k).and_then(|e| super::fields::Fields::of(&e.2))
    }
    /// The fields of the run's `k`-th version, a whole program's (the run's, a maintained form: `of_placed`, `push`).
    pub fn whole(&self, k: usize) -> super::fields::Fields<'_> {
        self.fields(k).unwrap_or_else(|| panic!("the run's version {k} is its rules alone"))
    }
    /// The rule `k` names, as the planner reads it.
    pub fn at(&self, k: RuleKey) -> Body<'_> {
        let mut b = self.0[k.v as usize].2.body(k.id as usize);
        // A maintained form's rules name their sources by the run's version's ids (`Body::origin`).
        if k.v != 0 {
            b.sources = Some(&self.0[0].2.v);
        }
        b
    }
    /// The rule `k` names by its handles, its literals read in place: for a reader that walks it once, with no
    /// `Body` (which collects the literals).
    pub fn rule_at(&self, k: RuleKey) -> (&Version, RuleRef<'_>) {
        let p = &self.0[k.v as usize].2;
        (&p.v, p.rule(k.id as usize))
    }
    /// The relation the rule `k` names derives (its head's).
    pub fn head_at(&self, k: RuleKey) -> &str {
        let (v, rule) = self.rule_at(k);
        v.rel_name(rule.head().rel())
    }
}

/// Rule `k` of version `v`, as the planner reads it.
pub fn nth(v: &Version, k: usize) -> Body<'_> {
    Body::of(v, v.rule(RuleId(k as u32)).expect("a made rule"))
}

#[cfg(test)]
mod tests {
    use super::super::super::program_image::{Builder, T, Version};
    use super::super::super::rr::parse_version;
    use super::{Freeze as _, Placed};

    /// 0276 §3: a writer frozen in an order of its own reads each rule where the pass put it, the edit folded or not.
    /// A fold numbers the rules afresh (`Builder::finish_placing`), so the order the pass wrote is read back through it.
    #[test]
    fn a_frozen_edit_keeps_its_order_through_a_fold() {
        let rels = ["a", "b", "c", "f", "g", "h", "i", "j"];
        let text: String = rels.iter().map(|r| format!(".decl {r}(x: number)\n{r}(x) :- e(x).\n")).collect();
        let v = parse_version(&format!(".decl e(x: number)\n.input e\n.decl d(x: number)\n{text}")).unwrap();
        let shown = |p: &Placed| (0..p.len()).map(|k| p.v.show_rule(p.rule(k))).collect::<Vec<_>>();
        let ids: Vec<_> = v.rules().map(|r| r.id()).collect();
        // One rule removed and two added: an overlay of the eight. Four removed: half the program or more, folded.
        for (remove, folds) in [(1, false), (4, true)] {
            let mut b = Builder::on(&v);
            for &id in &ids[..remove] {
                b.remove(id);
            }
            let x = b.rule().head("d", &[T::Var("x")]).atom("a", &[T::Var("x")]).finish().unwrap();
            let y = b.rule().head("d", &[T::Var("x")]).atom("b", &[T::Var("x")]).finish().unwrap();
            let p = b.freeze_placed(vec![y, ids[7], x], true).unwrap();
            assert_eq!(!matches!(&*p.v, Version::Overlay { .. }), folds, "{remove} removed: folded");
            assert_eq!(shown(&p), ["d($x) :- b($x)", "j($x) :- e($x)", "d($x) :- a($x)"], "{remove} removed");
        }
    }
}
