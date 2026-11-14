//! A rule as the planner reads it (decision 0173, t36 item 2): the rule's handles in its version, its literals by
//! position, and each literal's variables as a set of slots. The planner plans a rule, not a (head, body) pair, and
//! its steps name the rule's literals by position (`Step::at`), as every other reader of a plan does.
//!
//! Each reading here is the tree's (`front::Literal::vars`, `plan::ready`, `front::cmp_binds`, `plan::is_join`, the
//! digests and the text), stated over handles, and checked against it (`tests`): a plan searched over a `Body` is the
//! plan the tree's search made, and its keys (`Body::digest`, `Body::observed_key`) are the tree's keys.
use super::super::front::{Program, Rule};
use super::super::program_image::{AggRef, AtomRef, Builder, CmpOp, Lit, LitRef, RuleId, RuleRef, Slot, TermRef, Version};
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
}

impl<'v> Body<'v> {
    pub fn of(v: &'v Version, rule: RuleRef<'v>) -> Body<'v> {
        Body { v, rule, lits: rule.body().collect(), vars: OnceCell::new(), args: OnceCell::new() }
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
    /// Whether literal `i` assigns rather than filters: an equality or an aggregate (the search halves rows by every
    /// other filter).
    pub fn assigns(&self, i: usize) -> bool {
        matches!(self.view(i), Lit::Cmp { op: CmpOp::Eq, .. } | Lit::Agg { .. })
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
    /// The signature of literal `i`'s atom with `bound` bound, under which its fanouts are observed (`plan::atom_sig`).
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
    /// A constant term's value (`plan::constant`).
    pub fn constant(&self, t: TermRef<'_>) -> Option<Constant> {
        match t {
            TermRef::Int(i) => Some(Constant::Int(i)),
            TermRef::Str(a) => Some(Constant::Str(self.v.text(a).to_owned())),
            _ => None,
        }
    }
    /// Whether literal `i` joins: a positive atom, or `nearest` (`plan::is_join`).
    pub fn is_join(&self, i: usize) -> bool {
        match self.view(i) {
            Lit::Atom(a) => !a.neg(),
            Lit::Nearest { .. } => true,
            _ => false,
        }
    }
    /// Whether filter `i` can run with `bound` bound (`plan::ready`).
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

    /// The rule written into a digest, as `plan::digest_atom` and `plan::digest_literal` write the tree's head and body.
    pub(in crate::code_rule_dsl) fn digest(&self, d: &mut super::super::derived::Inputs) {
        self.digest_atom(d, self.head());
        d.u64(self.lits.len() as u64);
        for &l in &self.lits {
            self.digest_lit(d, l);
        }
    }
    /// The key observations of this rule's steps are kept under (`plan::obs_rule`).
    pub fn observed_key(&self) -> u64 {
        let d = super::super::derived::digest(|d| self.digest(d));
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

/// An equality of a bound variable or constant with an unbound variable assigns the right side (`plan::binds_right`).
pub(super) fn binds_right(l: TermRef<'_>, op: CmpOp, r: TermRef<'_>, bound: &VarSet) -> Option<Slot> {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OFF.get_or_init(|| std::env::var("RERAIL_EQUALITY_BINDS").is_ok_and(|v| v == "off")) || op != CmpOp::Eq {
        return None;
    }
    let TermRef::Var(y) = r else { return None };
    let ready = match l {
        TermRef::Var(x) => bound.contains(x),
        TermRef::Int(_) | TermRef::Str(_) => true,
        _ => false,
    };
    (ready && !bound.contains(y)).then_some(y)
}

/// The versions of the programs whose rules a run plans, each encoded once, and a rule of one found by its place in
/// the program's rules. Until `plan::run` takes a version (t36 item 7) and its rules are keyed by id (item 5), the run's
/// programs are trees, and this is how the planner reads their rules by handles. A program is added once it is
/// final: its rules do not move after.
#[derive(Clone, Default)]
pub struct Versions(Vec<(usize, usize, Arc<Version>)>);

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
    /// The rule's number in the program, a rule of the program's version (a maintained form's has none).
    pub fn number(self) -> Option<usize> {
        (self.v == 0).then_some(self.id as usize)
    }
}

impl Versions {
    /// The version of `prog` alone, the run's version 0 (rule numbers are its ids).
    pub fn of(prog: &Program) -> Result<Versions> {
        let rules = &prog.rules;
        let start = rules.as_ptr() as usize;
        Ok(Versions(vec![(start, start + std::mem::size_of_val(rules.as_slice()), made(rules)?)]))
    }
    /// Adds `prog`'s rules, encoded (the planner reads its rules alone; the rest of a program it reads from the tree
    /// until item 7, and a maintained form's parts have no image).
    pub fn add(&mut self, prog: &Program) -> Result<()> {
        self.add_rules(&prog.rules)
    }
    /// Adds `rules` (a program's, or a maintained form's), encoded, unless they are held already.
    pub fn add_rules(&mut self, rules: &[Rule]) -> Result<()> {
        let start = rules.as_ptr() as usize;
        if rules.is_empty() || self.0.iter().any(|e| e.0 == start) {
            return Ok(());
        }
        let v = made(rules)?;
        self.0.push((start, start + std::mem::size_of_val(rules), v));
        Ok(())
    }
    /// The version `r` is a rule of, and its id there.
    pub fn find(&self, r: &Rule) -> Option<(&Arc<Version>, RuleId)> {
        let k = self.key(r)?;
        Some((&self.0[k.v as usize].2, RuleId(k.id)))
    }
    /// `r`'s key, a rule of a program the run holds.
    pub fn key(&self, r: &Rule) -> Option<RuleKey> {
        let at = r as *const Rule as usize;
        let v = self.0.iter().position(|e| e.0 <= at && at < e.1)?;
        Some(RuleKey { v: v as u32, id: ((at - self.0[v].0) / std::mem::size_of::<Rule>()) as u32 })
    }
    /// `r`'s key, or an error naming it (a rule of no program the run holds).
    pub fn key_of(&self, r: &Rule) -> Result<RuleKey> {
        self.key(r).ok_or_else(|| Error::Plan(format!("{} :- …: a rule of no program the run holds", r.head)))
    }
    /// The rule `k` names, as the planner reads it.
    pub fn at(&self, k: RuleKey) -> Body<'_> {
        let v = &self.0[k.v as usize].2;
        Body::of(v, v.rule(RuleId(k.id)).expect("a rule of its version"))
    }
    /// Rule `r` as the planner reads it.
    pub fn body(&self, r: &Rule) -> Result<Body<'_>> {
        let (v, id) = self.find(r).ok_or_else(|| Error::Plan(format!("{} :- …: a rule of no program the run holds", r.head)))?;
        let rule = v.rule(id).ok_or_else(|| Error::Plan(format!("{}: rule {} is not in its version", r.head, id.0)))?;
        Ok(Body::of(v, rule))
    }
}

/// Rules made for planning alone (a rule's body less its recursive atom, a factored part, a view's atom), each a rule
/// of its own version, read as the planner reads any rule. The made rules are trees until their readers read handles
/// (t36 items 3 and 4); each is encoded as it is, so its literals keep their places.
pub fn made(rules: &[Rule]) -> Result<Arc<Version>> {
    let mut b = Builder::base();
    for r in rules {
        b.add(r).map_err(Error::Plan)?;
    }
    Ok(b.finish().map_err(Error::Plan)?.0)
}

/// Rule `k` of a version `made` made, as the planner reads it.
pub fn nth(v: &Version, k: usize) -> Body<'_> {
    Body::of(v, v.rule(RuleId(k as u32)).expect("a made rule"))
}

/// `rule` alone, a version of its own (a rule of no program the run holds: a copy a pass prices before it makes it).
pub fn alone(rule: &Rule) -> Result<Arc<Version>> {
    made(std::slice::from_ref(rule))
}
