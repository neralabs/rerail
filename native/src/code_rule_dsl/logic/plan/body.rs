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

/// The variables of term `t`, into `out` (`front::Term::vars`).
pub fn term_vars(t: TermRef<'_>, out: &mut VarSet) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

/// The variables of atom `a` (`front::Atom::vars`).
pub fn atom_vars(a: AtomRef<'_>) -> VarSet {
    let mut out = VarSet::new();
    a.args().for_each(|t| term_vars(t, &mut out));
    out
}

/// The variables of literal `l` (`front::Literal::vars`).
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
}

impl<'v> Body<'v> {
    pub fn of(v: &'v Version, rule: RuleRef<'v>) -> Body<'v> {
        Body { v, rule, lits: rule.body().collect(), vars: OnceCell::new(), args: OnceCell::new(), key: OnceCell::new(), sources: None }
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
                self.v.text(f) == o.v.text(g) && xs.clone().count() == ys.clone().count() && xs.zip(ys).all(|(x, y)| self.same_term(x, o, y))
            }
            _ => false,
        }
    }
    /// The arguments of literal `i`'s atom (its own, or its aggregate's; none for another literal).
    pub fn args(&self, i: usize) -> &[TermRef<'v>] {
        &self.args.get_or_init(|| self.lits.iter().map(|l| l.atom().map_or_else(Vec::new, |a| a.args().collect())).collect())[i]
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
    /// The demand of a demand-restricted aggregate (`rr_demand_agg_…`, a positive atom), which drives its rule.
    pub fn demand_first(&self) -> Option<usize> {
        (0..self.len()).find(|&i| self.plain_atom(i).is_some_and(|a| !a.neg() && self.rel(a).contains("rr_demand_agg_")))
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
    /// Whether literals `i` and `j` read equal atoms (`front::Atom`'s equality: relation, flags and arguments).
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
    /// The atom literal `i` reads: its own, or its aggregate's (`front::Literal::atom`).
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
    /// `@cat` over variables only (`front::Term::is_cat`).
    pub fn is_cat(&self, t: TermRef<'_>) -> bool {
        matches!(t, TermRef::Call(f, args) if self.v.text(f) == "cat" && { args.clone().all(|a| matches!(a, TermRef::Var(_))) })
    }

    /// The rule written into a derived value's inputs (`logic::derived`), field by field: its head and body, the same
    /// pieces in the same order as its text, without making the text.
    pub(in crate::code_rule_dsl) fn digest(&self, d: &mut super::super::derived::Inputs) {
        self.digest_atom(d, self.head());
        d.u64(self.lits.len() as u64);
        for &l in &self.lits {
            self.digest_lit(d, l);
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

    /// Literal `i` as text (`front::Literal`'s).
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
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OFF.get_or_init(|| std::env::var("RERAIL_EQUALITY_BINDS").is_ok_and(|v| v == "off")) || op != CmpOp::Eq {
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
pub struct Versions(Vec<(usize, usize, Placed)>, Vec<AgainAt>);

/// Where a maintained program's stratum's rules for deriving it again, or its deltas, are among a run's versions
/// (`Versions::add_again`): its `Maint` (by address), the stratum's place there, which list, and the version's place.
#[derive(Clone, Copy)]
struct AgainAt {
    maint: usize,
    group: usize,
    deltas: bool,
    at: usize,
}

/// A program's version, and where its rules are in it: the program's rule `k` is the version's rule `ids[k]` (rule
/// `k` where `ids` is `None`, a version made of the rules in order). A pass's overlay keeps its rules where its edits
/// put them (a reader's copies after the rules it does not touch), and the program, decoded, in the tree pass's order.
#[derive(Clone)]
pub struct Placed {
    pub v: Arc<Version>,
    ids: Option<Arc<[RuleId]>>,
    /// Whether `v` holds the program's other fields too (`of_program`), not its rules alone.
    pub whole: bool,
    /// A maintained program's parts (decision 0215): `None` for every other program.
    pub maint: Option<Arc<Maint>>,
    /// The version's other fields, read once at the first `Fields::of` and shared by every clone after it.
    pub(super) fields: std::sync::OnceLock<Arc<super::fields::FieldsData>>,
    /// The ids of `v`'s rules in order, made at the first `id` where `ids` is `None` and `v` numbers its rules other
    /// than `0..size` (a view or an overlay: a split's part, decision 0244).
    positions: std::sync::OnceLock<Arc<[RuleId]>>,
}

/// A maintained program's parts beside its version (decision 0215): each stratum that may be derived again
/// (`AgainRef`), its rules and deltas rules of `v`, an overlay of the program's version, and the sets of relations
/// run as one component (`together`). Every relation and rule is `v`'s.
#[derive(Clone)]
pub struct Maint {
    pub v: Arc<Version>,
    pub again: Vec<AgainRef>,
    pub together: Vec<Vec<RelId>>,
}

/// A stratum derived again, as a maintained form's writer states it (`Maint::written`): its relations (each set in
/// name order, as the tree's sets), its rules as given (rules of the form's source), its relations whose deltas
/// against their old rows it makes (with their widths), the input deltas it reads and its old rows.
pub struct Group {
    pub scc: Vec<String>,
    pub owned: Vec<String>,
    pub given: Vec<RuleId>,
    pub deltas: Vec<(String, usize)>,
    pub reads: Vec<String>,
    pub olds: Vec<String>,
}

/// A maintained stratum's other form (decision 0090 §4), derived again: its relations (one fixpoint when derived
/// again), every relation its maintained form makes for it (its relations, their pieces `__del`, `__ins`, … and
/// deltas), its rules over the current inputs and its deltas against its old rows, the input deltas it reads and its
/// old rows (the sizes that choose). Both forms make its relations and their deltas (`r__minus`, `r__plus`), which are
/// all that other strata read of it, so each stratum chooses alone; the run chooses once its input deltas are known.
#[derive(Clone, Debug)]
pub struct AgainRef {
    pub scc: Vec<RelId>,
    pub owned: Vec<RelId>,
    pub rules: Vec<RuleId>,
    pub deltas: Vec<RuleId>,
    pub reads: Vec<RelId>,
    pub olds: Vec<RelId>,
}

impl Maint {
    /// The parts' digest (decision 0173 §7): each stratum derived again (its relations by name, its rules' and deltas'
    /// hashes) and each set run as one component, summed, so it reads no rule's text. A program's digest with its
    /// version's is `Placed::digest`.
    pub fn digest(&self) -> u64 {
        use super::super::program_image::strings::hash;
        let names = |rels: &[RelId]| self.names(rels).collect::<Vec<_>>().join("\0");
        let rules = |ids: &[RuleId]| ids.iter().fold(0u64, |a, &id| a.wrapping_add(self.v.rule(id).map_or(0, |r| r.hash())));
        let mut d = 0u64;
        for g in &self.again {
            let text = format!("again\x01{}\x01{}\x01{}\x01{}\x01{:016x}\x01{:016x}", names(&g.scc), names(&g.owned), names(&g.reads), names(&g.olds), rules(&g.rules), rules(&g.deltas));
            d = d.wrapping_add(hash(text.as_bytes()));
        }
        for t in &self.together {
            d = d.wrapping_add(hash(format!("together\x01{}", names(t)).as_bytes()));
        }
        d
    }
    /// The name of relation `r`, one of `v`'s.
    pub fn name(&self, r: RelId) -> &str {
        self.v.rel_name(r)
    }
    /// The names of `rels`, a set of a stratum's (`AgainRef`), in order.
    pub fn names<'m>(&'m self, rels: &'m [RelId]) -> impl Iterator<Item = &'m str> + 'm {
        rels.iter().map(move |&r| self.v.rel_name(r))
    }
    /// The stratum derived again whose relations are `scc`, if one is.
    pub fn group_of(&self, scc: &std::collections::BTreeSet<String>) -> Option<usize> {
        self.again.iter().position(|g| g.scc.len() == scc.len() && self.names(&g.scc).zip(scc).all(|(a, b)| a == b))
    }
    /// Whether a set run as one component (`together`) holds `rel`.
    pub fn together_holds(&self, rel: &str) -> bool {
        self.v.rel(rel).is_some_and(|r| self.together.iter().any(|t| t.contains(&r)))
    }
    /// A maintained form's `again` and `together` (decision 0241), over `parent`, the form's derivation: each group's
    /// rules as given (`given`, rules of `src`), then its deltas against its old rows (`deltas`: each relation and its
    /// width, `r__minus` then `r__plus`), as `maintain` writes them.
    pub fn written(parent: &Arc<Version>, src: &Version, groups: &[Group], together: &[std::collections::BTreeSet<String>]) -> Result<Maint> {
        use super::super::program_image::T;
        let mut b = Builder::on(parent);
        let mut ids = Vec::with_capacity(groups.len());
        for g in groups {
            let rules: Vec<RuleId> = g.given.iter().map(|&id| b.copy(src, src.rule(id).expect("a given rule"))).collect();
            let mut deltas = Vec::with_capacity(2 * g.deltas.len());
            for (rel, width) in &g.deltas {
                let names: Vec<String> = (0..*width).map(|i| format!("_m{i}")).collect();
                let v: Vec<T<'_>> = names.iter().map(|n| T::Var(n)).collect();
                let (old, minus, plus) = (format!("{rel}__old"), format!("{rel}__minus"), format!("{rel}__plus"));
                deltas.push(b.rule().head(minus.as_str(), &v).atom(old.as_str(), &v).not(rel.as_str(), &v).finish().map_err(Error::Plan)?);
                deltas.push(b.rule().head(plus.as_str(), &v).atom(rel.as_str(), &v).not(old.as_str(), &v).finish().map_err(Error::Plan)?);
            }
            for n in g.scc.iter().chain(&g.owned).chain(&g.reads).chain(&g.olds) {
                b.rel(n);
            }
            ids.push((rules, deltas));
        }
        for s in together {
            for n in s {
                b.rel(n);
            }
        }
        // An overlay, never folded: its rules keep the ids they were written with.
        let v = b.finish_overlay().map_err(Error::Plan)?;
        let rel = |n: &str| v.rel(n).ok_or_else(|| Error::Plan(format!("a maintained form's relation {n} has no id")));
        let rels = |ns: &[String]| ns.iter().map(|n| rel(n)).collect::<Result<Vec<RelId>>>();
        let again = groups
            .iter()
            .zip(ids)
            .map(|(g, (rules, deltas))| Ok(AgainRef { scc: rels(&g.scc)?, owned: rels(&g.owned)?, rules, deltas, reads: rels(&g.reads)?, olds: rels(&g.olds)? }))
            .collect::<Result<Vec<_>>>()?;
        let together = together.iter().map(|s| s.iter().map(|n| rel(n)).collect::<Result<Vec<RelId>>>()).collect::<Result<Vec<_>>>()?;
        Ok(Maint { v, again, together })
    }
}

impl Placed {
    /// The run's version of the program whose image this is (t36 item 7, step 2): normalized for the SQL kernel
    /// (`passes::normalize`, a derivation), with its stated facts apart where `apart` (`passes::stated_apart`, an
    /// overlay), as `plan::run` normalizes the program. Also the relations given a presence column (the sentinels),
    /// and the pairs `(rel, rel__stated)`, by name.
    pub fn normalized(&self, apart: bool) -> Result<Normalized> {
        use super::super::program_image::passes;
        if !self.whole || self.ids.is_some() {
            return Err(Error::Plan("the program has no image: its run reads it from its version".into()));
        }
        let (v, sentinels) = passes::normalize(&self.v).map_err(Error::Plan)?;
        let sentinels = sentinels.iter().map(|&r| v.rel_name(r).to_owned()).collect();
        let (v, stated) = if apart { passes::stated_apart(&v).map_err(Error::Plan)? } else { (v, Vec::new()) };
        let stated = stated.iter().map(|&(r, s)| (v.rel_name(r).to_owned(), v.rel_name(s).to_owned())).collect();
        Ok(Normalized { placed: Placed { v, ids: None, whole: true, maint: self.maint.clone(), fields: Default::default(), positions: Default::default() }, sentinels, stated })
    }
    /// A maintained form, a derivation of the run's version (decision 0241): its rules `v`'s in order, its fields
    /// its own, and its maintenance's parts beside it.
    pub fn derived(v: Arc<Version>, maint: Option<Arc<Maint>>) -> Placed {
        Placed { v, ids: None, whole: true, maint, fields: Default::default(), positions: Default::default() }
    }
    /// Its components, dependencies first, in the tree's order (`analyses::components_by_name`), its maintained program's
    /// sets run together.
    pub fn components(&self) -> Result<Arc<super::super::program_image::analyses::Components>> {
        use super::super::program_image::analyses::Components;
        let f = super::fields::Fields::of(self).ok_or_else(|| Error::Plan("the components of a version of rules alone".into()))?;
        let together = self.together();
        if together.is_empty() {
            // Held by the version (decision 0281 §2): computed once however many readers and `Placed`s ask.
            return super::super::program_image::manager::analysis::<Components>(&self.v).map_err(Error::Plan);
        }
        Ok(Arc::new(Components(super::super::program_image::analyses::components_by_name(f.v, f.about, &together)?)))
    }
    /// The sets its maintained program runs as one component (`Maint::together`), by name.
    pub fn together(&self) -> Vec<std::collections::BTreeSet<String>> {
        self.maint.as_ref().map_or_else(Vec::new, |m| m.together.iter().map(|s| s.iter().map(|&r| m.v.rel_name(r).to_owned()).collect()).collect())
    }
    /// A version whose rules are `v`'s in order (a program's image).
    pub fn whole(v: Arc<Version>) -> Placed {
        Placed { v, ids: None, whole: true, maint: None, fields: Default::default(), positions: Default::default() }
    }
    /// `v`, the program's rule `k` its rule `ids[k]`.
    pub fn new(v: Arc<Version>, ids: Vec<RuleId>, whole: bool) -> Placed {
        Placed { v, ids: Some(ids.into()), whole, maint: None, fields: Default::default(), positions: Default::default() }
    }
    /// The program's digest (decision 0173 §7): its version's, kept as the version was made, less the rules the
    /// program leaves out of it (where it is some of its version's rules), with its maintained parts'. A sum, so it
    /// does not see the rules' order.
    pub fn digest(&self) -> u64 {
        let d = match &self.ids {
            None => self.v.digest(),
            Some(ids) => {
                let all = self.v.rules().fold(0u64, |a, r| a.wrapping_add(r.hash()));
                ids.iter().fold(self.v.digest().wrapping_sub(all), |a, &id| a.wrapping_add(self.v.rule(id).map_or(0, |r| r.hash())))
            }
        };
        self.maint.as_ref().map_or(d, |m| d.wrapping_add(m.digest()))
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
        Versions(vec![(0, 0, placed)], Vec::new())
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
    /// Adds the rules stratum `g` of `maint` (decision 0215) is derived again by (`deltas` false: its rules over the
    /// current reads) or its deltas against its old rows (`deltas` true), rules of `maint.v`, unless held already:
    /// their version's place (`None` for no rules). Rule `k` of the version is the list's `k`-th.
    pub fn add_again(&mut self, maint: &Arc<Maint>, g: usize, deltas: bool) -> Option<usize> {
        if let Some(at) = self.again_at(maint, g, deltas) {
            return Some(at);
        }
        let at = Arc::as_ptr(maint) as usize;
        let a = &maint.again[g];
        let ids = if deltas { &a.deltas } else { &a.rules };
        if ids.is_empty() {
            return None;
        }
        self.0.push((0, 0, Placed::new(Arc::clone(&maint.v), ids.clone(), false)));
        self.1.push(AgainAt { maint: at, group: g, deltas, at: self.0.len() - 1 });
        Some(self.0.len() - 1)
    }
    /// Where the run holds stratum `g` of `maint`'s rules derived again, or its deltas (`add_again`), if it does.
    pub fn again_at(&self, maint: &Arc<Maint>, g: usize, deltas: bool) -> Option<usize> {
        let at = Arc::as_ptr(maint) as usize;
        self.1.iter().find(|e| e.maint == at && e.group == g && e.deltas == deltas).map(|e| e.at)
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
