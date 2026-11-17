//! `rr::lower::sources_on_request` on the version (decision 0174, 0281's path, step 7a-1): each per-node source
//! attribute of a round of `ON_REQUEST` answered on request, its readers asking for the nodes they bind, and the
//! relations that read an attribute read through their widened copies (`Widen`). See there for what each part does
//! and why: this is that pass over the version's rules, each rule it makes written from the reader it is made of.

use super::simplify::Tm;
use crate::code_rule_dsl::logic::front::{List, Value};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, Site, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomId, Builder, CmpOp, Lit, LitRef, RuleId, RuleRef, Slot, TermRef, Version};
use crate::code_rule_dsl::logic::rr::lower::{Answer, OnRequest};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;

/// `sources_on_request` of one round of `ON_REQUEST`.
pub(crate) struct SourcesOnRequest {
    pub(crate) round: &'static [OnRequest],
}

/// A literal of a rule made from a reader: one of the reader's own, or an atom read through its widened copy on
/// columns `cols`.
#[derive(Clone)]
enum Item {
    Own(u32),
    Wide { node: u32, rel: String, cols: Vec<usize> },
}

/// A rule made from rule `src`: a request (`req(n)`), a demand's ask (`req_by_r_c(rr_dn)`, decision 0291) or a widened
/// copy (its head's columns `cols`), and its body. Its variables `wild` are written as wildcards in its positive atoms
/// (`wild_singletons`); a demand's ask is written with its variables renamed apart (`rename`: the reading rule's
/// index and literal's, and the variable that is `rr_dn`); a request narrowed by a demand ends in its atom.
#[derive(Clone)]
struct New {
    src: RuleId,
    head: String,
    args: Head,
    body: Vec<Item>,
    wild: BTreeSet<Slot>,
    rename: Option<(usize, usize, Slot)>,
    demand: Option<String>,
}

#[derive(Clone)]
enum Head {
    Node(Slot),
    Cols(Vec<usize>),
}

fn lit_at<'v>(r: RuleRef<'v>, node: u32) -> LitRef<'v> {
    r.body().find(|l| l.node() == node).expect("a literal of the rule")
}

fn term_vars(t: TermRef<'_>, out: &mut BTreeSet<Slot>) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

/// A literal's variables (`Literal::vars`).
fn lit_vars(l: LitRef<'_>) -> BTreeSet<Slot> {
    let mut out = BTreeSet::new();
    match l.view() {
        Lit::Atom(a) => a.args().for_each(|t| term_vars(t, &mut out)),
        Lit::Cmp { l, r, .. } => {
            term_vars(l, &mut out);
            term_vars(r, &mut out);
        }
        Lit::Agg { var, over, atom, .. } => {
            atom.args().for_each(|t| term_vars(t, &mut out));
            out.insert(var);
            out.extend(over);
        }
        Lit::Nearest { n, a, .. } => out.extend([n, a]),
        Lit::Ast { document, node, captures, .. } => {
            out.extend([document, node]);
            out.extend(captures.map(|(_, s)| s));
        }
    }
    out
}

/// A demand's ask's variables renamed apart, and a rule's wildcards in its positive atoms (`New::write`).
struct Names<'a> {
    wild: &'a BTreeSet<Slot>,
    names: &'a BTreeMap<Slot, AtomId>,
}

impl Rewrite for Names<'_> {
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        match (site, t) {
            (Site::Atom(a), TermRef::Var(s)) if !a.neg() && self.wild.contains(&s) => Some(T::Wild),
            _ => None,
        }
    }
    fn var(&mut self, s: Slot) -> Option<AtomId> {
        self.names.get(&s).copied()
    }
}

/// An item's atom as written: its relation, whether negated, and its arguments, `None` for a wildcard (one of `wild`,
/// in a positive atom). `None` for a literal that is no atom.
fn written(v: &Version, r: RuleRef<'_>, item: &Item, wild: &BTreeSet<Slot>) -> Option<(String, bool, Vec<Option<String>>)> {
    let arg = |t: TermRef<'_>, neg: bool| match t {
        TermRef::Wild => None,
        TermRef::Var(s) if !neg && wild.contains(&s) => None,
        t => Some(v.show_term(r, t)),
    };
    match item {
        Item::Own(node) => {
            let Lit::Atom(a) = lit_at(r, *node).view() else { return None };
            Some((v.rel_name(a.rel()).to_owned(), a.neg(), a.args().map(|t| arg(t, a.neg())).collect()))
        }
        Item::Wide { node, rel, cols } => {
            let Lit::Atom(a) = lit_at(r, *node).view() else { unreachable!("a widened atom") };
            Some((rel.clone(), false, cols.iter().map(|&c| arg(a.args().nth(c).expect("a column"), false)).collect()))
        }
    }
}

/// `rr::lower::wild_singletons` over items: the variables, other than `head`, that only one atom reads, once (its
/// written columns: a widened atom's kept ones), as wildcards; and without each positive atom another positive one of
/// the same relation is an instance of. To a fixpoint (decision 0291).
fn wild_singletons(v: &Version, r: RuleRef<'_>, mut body: Vec<Item>, head: Slot) -> (Vec<Item>, BTreeSet<Slot>) {
    fn count(t: TermRef<'_>, n: &mut BTreeMap<Slot, usize>) {
        match t {
            TermRef::Var(s) => *n.entry(s).or_default() += 1,
            TermRef::Call(_, args) => args.for_each(|a| count(a, n)),
            _ => {}
        }
    }
    loop {
        let mut n: BTreeMap<Slot, usize> = BTreeMap::new();
        for item in &body {
            match item {
                Item::Own(node) => match lit_at(r, *node).view() {
                    Lit::Atom(a) => a.args().for_each(|t| count(t, &mut n)),
                    // Any other literal's variables are read where they are, never wildcards.
                    _ => lit_vars(lit_at(r, *node)).into_iter().for_each(|s| *n.entry(s).or_default() += 2),
                },
                Item::Wide { node, cols, .. } => {
                    let Lit::Atom(a) = lit_at(r, *node).view() else { unreachable!("a widened atom") };
                    cols.iter().for_each(|&c| count(a.args().nth(c).expect("a column"), &mut n));
                }
            }
        }
        let wild: BTreeSet<Slot> = n.into_iter().filter(|&(s, c)| s != head && c == 1).map(|(s, _)| s).collect();
        let atoms: Vec<Option<(String, bool, Vec<Option<String>>)>> = body.iter().map(|i| written(v, r, i, &wild)).collect();
        let instance = |a: &(String, bool, Vec<Option<String>>), b: &(String, bool, Vec<Option<String>>)| {
            a.0 == b.0 && !b.1 && a.2.len() == b.2.len() && a.2.iter().zip(&b.2).all(|(x, y)| x.is_none() || x == y)
        };
        let Some(i) = (0..body.len()).find(|&i| {
            matches!(&atoms[i], Some(a) if !a.1 && (0..body.len()).any(|j| j != i && matches!(&atoms[j], Some(b) if instance(a, b) && !(instance(b, a) && j > i))))
        }) else {
            return (body, wild);
        };
        body.remove(i);
    }
}

impl New {
    /// A rule with its body as it is: no wildcards, no renames, no demand.
    fn plain(src: RuleId, head: String, args: Head, body: Vec<Item>) -> New {
        New { src, head, args, body, wild: BTreeSet::new(), rename: None, demand: None }
    }
    /// A variable's name as written.
    fn name(&self, v: &Version, r: RuleRef<'_>, s: Slot) -> String {
        match self.rename {
            Some((_, _, u)) if s == u => "rr_dn".to_owned(),
            Some((k, i, _)) => format!("{}_rr_d{k}_{i}", v.text(r.var(s))),
            None => v.text(r.var(s)).to_owned(),
        }
    }
    fn src<'v>(&self, v: &'v Version) -> RuleRef<'v> {
        v.rule(self.src).expect("a rule of the input")
    }
    /// The rule as text: what tells two made rules apart (`Rule`'s equality).
    fn key(&self, v: &Version) -> String {
        let r = self.src(v);
        let head = match &self.args {
            Head::Node(n) => self.name(v, r, *n),
            Head::Cols(cols) => cols.iter().map(|&c| v.show_term(r, r.head().args().nth(c).expect("a head column"))).collect::<Vec<_>>().join(", "),
        };
        let mut body: Vec<String> = self
            .body
            .iter()
            .map(|i| match written(v, r, i, &self.wild) {
                Some((rel, neg, args)) => format!("{}{rel}({})", if neg { "!" } else { "" }, args.iter().map(|a| a.as_deref().unwrap_or("_")).collect::<Vec<_>>().join(", ")),
                None => v.show_lit(r, lit_at(r, match i { Item::Own(node) => *node, Item::Wide { node, .. } => *node })),
            })
            .collect();
        if let (Some(dem), Head::Node(n)) = (&self.demand, &self.args) {
            body.push(format!("{dem}({})", self.name(v, r, *n)));
        }
        format!("{}({head}) :- {}", self.head, body.join(", "))
    }
    /// The relations its body reads.
    fn reads(&self, v: &Version) -> Vec<String> {
        let r = self.src(v);
        self.body
            .iter()
            .filter_map(|i| match i {
                Item::Own(node) => lit_at(r, *node).atom().map(|a| v.rel_name(a.rel()).to_owned()),
                Item::Wide { rel, .. } => Some(rel.clone()),
            })
            .collect()
    }
    fn write(&self, v: &Version, b: &mut Builder<'_>) -> Result<()> {
        let r = self.src(v);
        // The variables renamed, interned before the rule is opened.
        let mut names: BTreeMap<Slot, AtomId> = BTreeMap::new();
        if self.rename.is_some() {
            for i in &self.body {
                let node = match i {
                    Item::Own(node) | Item::Wide { node, .. } => *node,
                };
                for s in lit_vars(lit_at(r, node)) {
                    let name = self.name(v, r, s);
                    names.entry(s).or_insert_with(|| b.atom(&name));
                }
            }
        }
        let demand = match (&self.demand, &self.args) {
            (Some(dem), Head::Node(n)) => Some((dem.as_str(), b.atom(&self.name(v, r, *n)))),
            _ => None,
        };
        let head: Vec<T<'static>> = match &self.args {
            Head::Node(n) => vec![T::VarAtom(b.atom(&self.name(v, r, *n)))],
            Head::Cols(cols) => cols.iter().map(|&c| Tm::of(v, r, r.head().args().nth(c).expect("a head column")).write(b)).collect(),
        };
        let wide: Vec<Option<Vec<T<'static>>>> = self
            .body
            .iter()
            .map(|i| match i {
                Item::Own(_) => None,
                Item::Wide { node, cols, .. } => {
                    let Lit::Atom(a) = lit_at(r, *node).view() else { unreachable!("a widened atom") };
                    Some(
                        cols.iter()
                            .map(|&c| match a.args().nth(c).expect("a column") {
                                TermRef::Var(s) if self.wild.contains(&s) => T::Wild,
                                t => Tm::of(v, r, t).write(b),
                            })
                            .collect(),
                    )
                }
            })
            .collect();
        let plain = self.wild.is_empty() && names.is_empty();
        let mut rw = Names { wild: &self.wild, names: &names };
        let mut out = b.rule().head(self.head.as_str(), &head);
        for (i, args) in self.body.iter().zip(&wide) {
            out = match (i, args) {
                (Item::Own(node), _) if plain => out.lit(v, r, lit_at(r, *node)),
                (Item::Own(node), _) => out.lit_with(v, r, lit_at(r, *node), &mut rw),
                (Item::Wide { rel, .. }, Some(args)) => out.atom_with(rel.as_str(), args, false, false),
                (Item::Wide { .. }, None) => unreachable!("a widened atom's arguments"),
            };
        }
        if let Some((dem, n)) = demand {
            out = out.atom(dem, &[T::VarAtom(n)]);
        }
        out.finish()?;
        Ok(())
    }
}

/// `Widen` over the version: the relations that read an attribute, and the columns their widened copies keep.
struct Widen<'v> {
    v: &'v Version,
    attrs: BTreeSet<&'static str>,
    prefix: String,
    cols: BTreeMap<String, Vec<usize>>,
}

/// What `Widen::rules` adds.
#[derive(Default)]
struct Widened {
    rules: Vec<New>,
    decls: BTreeMap<String, Vec<(String, String)>>,
    facts: BTreeMap<String, Vec<Vec<Value>>>,
    copies: Vec<(String, Vec<usize>)>,
    asked: BTreeMap<String, Vec<BTreeSet<String>>>,
}

impl<'v> Widen<'v> {
    fn new(v: &'v Version, attrs: BTreeSet<&'static str>) -> Self {
        let mut reading: BTreeSet<String> = attrs.iter().map(|a| a.to_string()).collect();
        loop {
            let before = reading.len();
            for r in v.rules() {
                if r.reads().any(|a| reading.contains(v.rel_name(a.rel()))) {
                    reading.insert(v.rel_name(r.head().rel()).to_owned());
                }
            }
            if reading.len() == before {
                break;
            }
        }
        attrs.iter().for_each(|a| {
            reading.remove(*a);
        });
        let prefix = format!("rr_wide_{}_", attrs.iter().next().copied().unwrap_or_default());
        let mut w = Widen { v, attrs, prefix, cols: BTreeMap::new() };
        w.cols = reading.iter().map(|r| (r.clone(), (0..w.arity(r)).collect())).collect();
        loop {
            let mut changed = false;
            for rule in v.rules() {
                let head = v.rel_name(rule.head().rel());
                let Some(cols) = w.cols.get(head) else { continue };
                let (_, bound, _) = w.widen(rule, None, &BTreeSet::new(), None);
                let kept: Vec<usize> = cols.iter().copied().filter(|&c| Self::bound_term(rule.head().args().nth(c).expect("a head column"), &bound)).collect();
                if kept.len() != cols.len() {
                    w.cols.insert(head.to_owned(), kept);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        w
    }

    fn arity(&self, rel: &str) -> usize {
        self.v.rel(rel).and_then(|id| self.v.heads(id).next()).map_or(0, |r| r.head().arity())
    }

    fn bound_term(t: TermRef<'_>, bound: &BTreeSet<Slot>) -> bool {
        let mut vars = BTreeSet::new();
        term_vars(t, &mut vars);
        !matches!(t, TermRef::Wild | TermRef::Top) && vars.is_subset(bound)
    }

    fn wide(&self, rel: &str) -> String {
        format!("{}{rel}", self.prefix)
    }

    fn left_out(dropped: &[(String, &'static str)]) -> String {
        if dropped.is_empty() {
            return String::new();
        }
        format!("; left out: {}", dropped.iter().map(|(l, why)| format!("`{l}` ({why})")).collect::<Vec<_>>().join(", "))
    }

    /// `Widen::widen` of rule `r`'s body: the items it keeps, the variables they bind, and what it left out.
    fn widen(&self, r: RuleRef<'_>, skip: Option<usize>, out: &BTreeSet<Slot>, to: Option<Slot>) -> (Vec<Item>, BTreeSet<Slot>, Vec<(String, &'static str)>) {
        let v = self.v;
        let mut dropped: Vec<(String, &'static str)> = Vec::new();
        // Each item, with its variables and whether it binds (an atom or a matcher).
        let mut lits: Vec<(Item, BTreeSet<Slot>, LitRef<'_>)> = Vec::new();
        let show = |l: LitRef<'_>| v.show_lit(r, l);
        for (j, l) in r.body().enumerate() {
            if Some(j) == skip {
                continue;
            }
            let vars = lit_vars(l);
            if !vars.is_disjoint(out) {
                dropped.push((show(l), "reads the value"));
                continue;
            }
            match l.view() {
                Lit::Atom(a) if a.neg() => dropped.push((show(l), "a negation")),
                Lit::Atom(a) if self.attrs.contains(v.rel_name(a.rel())) => dropped.push((show(l), "an attribute on request")),
                Lit::Atom(a) => match self.cols.get(v.rel_name(a.rel())) {
                    Some(cols) if cols.is_empty() => dropped.push((show(l), "its widened copy keeps no column")),
                    Some(cols) => {
                        let mut kept = BTreeSet::new();
                        for &c in cols {
                            term_vars(a.args().nth(c).expect("a column"), &mut kept);
                        }
                        lits.push((Item::Wide { node: l.node(), rel: self.wide(v.rel_name(a.rel())), cols: cols.clone() }, kept, l));
                    }
                    None => lits.push((Item::Own(l.node()), vars, l)),
                },
                Lit::Agg { .. } => dropped.push((show(l), "an aggregate")),
                _ => lits.push((Item::Own(l.node()), vars, l)),
            }
        }
        if let Some(n) = to {
            let mut keep = vec![false; lits.len()];
            let mut reach: BTreeSet<Slot> = BTreeSet::from([n]);
            loop {
                let mut grew = false;
                for ((_, vars, _), k) in lits.iter().zip(keep.iter_mut()) {
                    if !*k && !vars.is_disjoint(&reach) {
                        *k = true;
                        reach.extend(vars.iter().copied());
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let mut keep = keep.into_iter();
            lits.retain(|(_, _, l)| {
                let k = keep.next().unwrap_or(false);
                if !k {
                    dropped.push((show(*l), "not joined to the node"));
                }
                k
            });
        }
        let binds = |l: LitRef<'_>| matches!(l.view(), Lit::Atom(_) | Lit::Ast { .. });
        let mut bound: BTreeSet<Slot> = lits.iter().filter(|(_, _, l)| binds(*l)).flat_map(|(_, vars, _)| vars.iter().copied()).collect();
        let (mut kept, mut left): (Vec<_>, Vec<_>) = lits.into_iter().partition(|(_, _, l)| binds(*l));
        loop {
            let before = kept.len();
            left.retain(|(item, vars, l)| {
                let unbound: Vec<Slot> = vars.iter().copied().filter(|x| !bound.contains(x)).collect();
                let ok = match l.view() {
                    Lit::Cmp { l: x, op, r: y } => unbound.is_empty() || op == CmpOp::Eq && unbound.len() == 1 && [x, y].iter().any(|t| t.var() == Some(unbound[0])),
                    Lit::Nearest { n, .. } => bound.contains(&n),
                    _ => false,
                };
                if ok {
                    bound.extend(vars.iter().copied());
                    kept.push((item.clone(), vars.clone(), *l));
                }
                !ok
            });
            if kept.len() == before {
                break;
            }
        }
        dropped.extend(left.iter().map(|(_, _, l)| (show(*l), "reads what nothing kept binds")));
        (kept.into_iter().map(|(i, _, _)| i).collect(), bound, dropped)
    }

    /// `Widen::demand_of` (decision 0291): what the rules reading `rule`'s relation ask of its column holding `n`, where
    /// every one asks: the column, and per reading site an ask (its head left to the caller), its other literals
    /// widened and joined to the column's variable, renamed apart with that variable as `rr_dn`. None where the
    /// relation is an output or an input, is read in an aggregate, by name (a kind set) or by its own rules, or is read
    /// with that column a constant or bound by nothing its reader keeps.
    fn demand_of(&self, rule: RuleRef<'_>, n: Slot) -> Option<(usize, Vec<New>)> {
        let v = self.v;
        let rel = v.rel_name(rule.head().rel());
        let col = rule.head().args().position(|t| t.var() == Some(n))?;
        if v.outputs().any(|o| v.text(o) == rel) || v.inputs().any(|i| v.text(i) == rel) {
            return None;
        }
        let mut asks = Vec::new();
        for (k, r) in v.rules().enumerate() {
            for (i, l) in r.body().enumerate() {
                let a = match l.view() {
                    Lit::Atom(a) if v.rel_name(a.rel()) == rel => a,
                    Lit::Agg { atom, .. } if v.rel_name(atom.rel()) == rel => return None,
                    Lit::Nearest { kindset, .. } if v.text(kindset) == rel => return None,
                    _ => continue,
                };
                if v.rel_name(r.head().rel()) == rel {
                    return None;
                }
                let Some(TermRef::Var(u)) = a.args().nth(col) else { return None };
                let (body, ..) = self.widen(r, Some(i), &BTreeSet::new(), Some(u));
                // No widened copy: a copy of a recursive relation can lose what bounds it. Its atoms are left out,
                // which only widens the ask; what is left must still bind the column and stay joined to it.
                let body = Self::joined(r, body.into_iter().filter(|it| matches!(it, Item::Own(_))).collect(), u);
                let binds = |it: &Item| matches!(it, Item::Own(node) if matches!(lit_at(r, *node).view(), Lit::Atom(_)) && lit_vars(lit_at(r, *node)).contains(&u));
                if !body.iter().any(binds) {
                    return None;
                }
                let (body, wild) = wild_singletons(v, r, body, u);
                asks.push(New { src: r.id(), head: String::new(), args: Head::Node(u), body, wild, rename: Some((k, i, u)), demand: None });
            }
        }
        (!asks.is_empty()).then_some((col, asks))
    }

    /// The atoms (and matchers) of `items` joined to `n` through atoms, then the comparisons over what they bind.
    fn joined(r: RuleRef<'_>, items: Vec<Item>, n: Slot) -> Vec<Item> {
        let lit = |it: &Item| match it {
            Item::Own(node) | Item::Wide { node, .. } => lit_at(r, *node),
        };
        let (atoms, rest): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| matches!(lit(it).view(), Lit::Atom(_) | Lit::Ast { .. }));
        let mut reach: BTreeSet<Slot> = BTreeSet::from([n]);
        let mut keep = vec![false; atoms.len()];
        loop {
            let mut grew = false;
            for (it, k) in atoms.iter().zip(keep.iter_mut()) {
                let vars = lit_vars(lit(it));
                if !*k && !vars.is_disjoint(&reach) {
                    *k = true;
                    reach.extend(vars);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut keep = keep.into_iter();
        let mut out: Vec<Item> = atoms.into_iter().filter(|_| keep.next().unwrap_or(false)).collect();
        out.extend(rest.into_iter().filter(|it| matches!(lit(it).view(), Lit::Cmp { .. }) && lit_vars(lit(it)).is_subset(&reach)));
        out
    }

    /// The widened copies `rules` read, and those they read in turn.
    fn rules(&self, rules: &[New], prog: &crate::code_rule_dsl::logic::front::Program) -> Widened {
        let v = self.v;
        let mut out = Widened::default();
        let reads = |rules: &[New]| -> Vec<String> { rules.iter().flat_map(|r| r.reads(v)).filter_map(|rel| rel.strip_prefix(self.prefix.as_str()).map(str::to_owned)).collect() };
        let mut todo = reads(rules);
        let mut done: BTreeSet<String> = BTreeSet::new();
        while let Some(rel) = todo.pop() {
            if !done.insert(rel.clone()) {
                continue;
            }
            let cols = &self.cols[&rel];
            let wide = self.wide(&rel);
            let decl = prog.decls.get(rel.as_str());
            out.decls.insert(wide.clone(), cols.iter().map(|&c| decl.and_then(|d| d.get(c)).map(|(n, t)| (n.to_string(), t.to_string())).unwrap_or_else(|| (format!("c{c}"), "symbol".to_owned()))).collect());
            if let Some(rows) = prog.facts.get(&rel) {
                out.facts.insert(wide.clone(), rows.iter().map(|row| cols.iter().map(|&c| row[c].clone()).collect()).collect());
            }
            let of: Vec<RuleRef<'_>> = v.rel(&rel).into_iter().flat_map(|id| v.heads(id)).collect();
            let asked = prog.asked_by.get(&rel).filter(|by| by.len() == of.len());
            let mut made: Vec<New> = Vec::new();
            let mut keys: Vec<String> = Vec::new();
            let mut readers: Vec<BTreeSet<String>> = Vec::new();
            for (k, rule) in of.iter().enumerate() {
                let (body, ..) = self.widen(*rule, None, &BTreeSet::new(), None);
                let r = New::plain(rule.id(), wide.clone(), Head::Cols(cols.clone()), body);
                let key = r.key(v);
                let by = asked.map_or_else(|| BTreeSet::from([wide.clone()]), |a| a[k].clone());
                match keys.iter().position(|m| *m == key) {
                    Some(at) => readers[at].extend(by),
                    None => {
                        made.push(r);
                        keys.push(key);
                        readers.push(by);
                    }
                }
            }
            if asked.is_some() {
                out.asked.insert(wide.clone(), readers);
            }
            todo.extend(reads(&made));
            out.rules.extend(made);
            out.copies.push((rel, cols.clone()));
        }
        out
    }
}

impl ProgramPass for SourcesOnRequest {
    const NAME: &'static str = "sources on request";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        use crate::code_rule_dsl::logic::profile::{decision, on};
        let v = cx.v();
        let inputs: Vec<&str> = v.inputs().map(|a| v.text(a)).collect();
        let attrs: Vec<OnRequest> = self.round.iter().copied().filter(|a| inputs.contains(&a.rel)).collect();
        if attrs.is_empty() {
            return Ok(());
        }
        let mut prog = v.header(&crate::code_rule_dsl::logic::front::Lists::default())?;
        let was = prog.clone();
        let applied = !std::env::var("RERAIL_NODE_SOURCES").is_ok_and(|x| x == "eager");
        let wd = Widen::new(v, attrs.iter().map(|a| a.rel).collect());
        let mut all: Vec<New> = Vec::new();
        let mut answered: Vec<OnRequest> = Vec::new();
        let mut asked: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
        // The readers' asks that narrow requests (decision 0291), by relation: each its rules, one per reading site.
        let mut demands: BTreeMap<String, Vec<New>> = BTreeMap::new();
        'attr: for &attr in &attrs {
            let (rel, req) = (attr.rel, attr.req);
            let topic = format!("{rel} on request");
            let mut requests: Vec<New> = Vec::new();
            let mut keys: Vec<String> = Vec::new();
            let mut readers: Vec<BTreeSet<String>> = Vec::new();
            let mut explained: Vec<String> = Vec::new();
            for rule in v.rules() {
                let reader = v.rel_name(rule.head().rel());
                for (i, lit) in rule.body().enumerate() {
                    let shown = || v.show_lit(rule, lit);
                    let eager = |why: &str| decision(&topic, || format!("no, every node: {reader} {why}{}", if applied { "" } else { " (RERAIL_NODE_SOURCES=eager)" }));
                    let a = match lit.view() {
                        Lit::Atom(a) if v.rel_name(a.rel()) == rel => a,
                        Lit::Agg { atom, .. } if v.rel_name(atom.rel()) == rel => {
                            eager(&format!("reads `{}` in an aggregate", shown()));
                            continue 'attr;
                        }
                        _ => continue,
                    };
                    let Some(TermRef::Var(n)) = a.args().next() else {
                        eager(&format!("reads `{}`, of a constant node", shown()));
                        continue 'attr;
                    };
                    let mut out: BTreeSet<Slot> = BTreeSet::new();
                    a.args().skip(1).for_each(|t| term_vars(t, &mut out));
                    out.remove(&n);
                    loop {
                        let before = out.len();
                        for l in rule.body() {
                            if let Lit::Cmp { l: x, op: CmpOp::Eq, r: y } = l.view() {
                                for (s, t) in [(x, y), (y, x)] {
                                    let mut vars = BTreeSet::new();
                                    term_vars(t, &mut vars);
                                    if let TermRef::Var(s) = s
                                        && !vars.is_disjoint(&out)
                                        && s != n
                                    {
                                        out.insert(s);
                                    }
                                }
                            }
                        }
                        if out.len() == before {
                            break;
                        }
                    }
                    let (body, bound, dropped) = wd.widen(rule, Some(i), &out, Some(n));
                    if !bound.contains(&n) {
                        eager(&format!("reads `{}`, and no literal it keeps binds ${}{}", shown(), v.text(rule.var(n)), Widen::left_out(&dropped)));
                        continue 'attr;
                    }
                    // Narrowed by what the reader's own readers ask of it (decision 0291): where `n` is a column of the
                    // reader's head that every rule reading it binds, the request joins those rules' asks.
                    let narrowed = wd.demand_of(rule, n).map(|(col, asks)| {
                        let dem = format!("{req}_by_{reader}_{col}");
                        demands.entry(dem.clone()).or_insert_with(|| asks.into_iter().map(|a| New { head: dem.clone(), ..a }).collect());
                        dem
                    });
                    // A variable the request reads once, outside its head, is a witness: a wildcard.
                    let (body, wild) = wild_singletons(v, rule, body, n);
                    let request = New { src: rule.id(), head: req.to_owned(), args: Head::Node(n), body, wild, rename: None, demand: narrowed.clone() };
                    let key = request.key(v);
                    if on() {
                        explained.push(format!("{reader} reads `{}`: asks `{key}`{}{}", shown(), Widen::left_out(&dropped), narrowed.map_or(String::new(), |d| format!("; narrowed by its readers' asks, `{d}`"))));
                    }
                    match keys.iter().position(|k| *k == key) {
                        Some(k) => {
                            readers[k].insert(reader.to_owned());
                        }
                        None => {
                            requests.push(request);
                            keys.push(key);
                            readers.push(BTreeSet::from([reader.to_owned()]));
                        }
                    }
                }
            }
            decision(&topic, || match explained.len() {
                0 => "no rule reads it: it stays an input, which a run that reads nothing of it binds for no node".to_owned(),
                n => format!("{}: {n} readers ask, by {} request rules", if applied { "yes" } else { "possible (RERAIL_NODE_SOURCES=eager)" }, requests.len()),
            });
            // Read by no rule, an attribute has nothing to answer, and stays an input (`sources_on_request`).
            if requests.is_empty() {
                continue;
            }
            for e in explained {
                decision(&format!("{rel} on request: readers"), || e);
            }
            all.extend(requests);
            if !readers.is_empty() {
                asked.insert(req.to_owned(), readers);
            }
            answered.push(attr);
        }
        let demanded: Vec<String> = demands.keys().cloned().collect();
        all.extend(std::mem::take(&mut demands).into_values().flatten());
        let wide = wd.rules(&all, &prog);
        for (rel, cols) in &wide.copies {
            decision("on request: widened copies", || format!("{} of {rel}, columns {cols:?} of {}{}", wd.wide(rel), wd.arity(rel), if cols.len() < wd.arity(rel) { " (the others are bound only by what the copy leaves out)" } else { "" }));
        }
        if !applied {
            return Ok(());
        }
        // The answers: each attribute's rows on request, the eager rows' rule, the named children.
        enum Answering {
            Eager { rel: &'static str, input: &'static str, n: usize },
            Function { rel: &'static str, req: &'static str, function: &'static str, n: usize },
            Children { rel: &'static str, req: &'static str, named: &'static str, n: usize },
        }
        let mut answers: Vec<Answering> = Vec::new();
        for OnRequest { rel, req, answer, eager } in answered {
            let cols = prog.decls.get(rel).cloned().unwrap_or_else(|| vec![("n".into(), "node".into()), ("v".into(), "symbol".into())].into());
            let n = cols.len();
            if let Some(input) = eager {
                answers.push(Answering::Eager { rel, input, n });
                prog.decls.insert(input.into(), cols.clone());
                prog.inputs.push(input.to_owned());
            }
            match answer {
                Answer::Function(function) => answers.push(Answering::Function { rel, req, function, n }),
                Answer::Children { named } => {
                    answers.push(Answering::Children { rel, req, named, n });
                    let mut named_cols = cols.clone();
                    named_cols.make_mut()[2] = ("c".into(), "node".into());
                    prog.decls.insert(named.into(), named_cols);
                    prog.inputs.push(named.to_owned());
                    if !prog.inputs.iter().any(|i| i == "node_text") {
                        prog.decls.entry("node_text".into()).or_insert_with(|| vec![("n".into(), cols[0].1.clone()), ("s".into(), cols[2].1.clone())].into());
                        prog.inputs.push("node_text".to_owned());
                    }
                }
            }
            prog.decls.insert(req.into(), vec![(cols[0].0.clone(), "node".into())].into());
            prog.inputs.retain(|i| i != rel);
        }
        prog.decls.extend(wide.decls.into_iter().map(|(r, cols)| (r.into(), cols.into_iter().map(|(n, t)| (n.into(), t.into())).collect())));
        for dem in demanded {
            prog.decls.insert(dem.as_str().into(), vec![("n".into(), "node".into())].into());
        }
        for (rel, rows) in wide.facts {
            prog.facts.entry(rel).or_default().extend(rows.into_iter().map(List::from));
        }
        prog.asked_by.extend(asked);
        prog.asked_by.extend(wide.asked);
        let b = w.on()?;
        b.set_header(&was, &prog);
        // In the tree's order: the requests and answers, then the widened copies.
        let names: Vec<String> = (0..answers.iter().map(|a| match a { Answering::Eager { n, .. } | Answering::Function { n, .. } | Answering::Children { n, .. } => *n }).max().unwrap_or(0)).map(|i| format!("x{i}")).collect();
        let x = |i: usize| T::Var(names[i].as_str());
        for r in &all {
            r.write(v, b)?;
        }
        for a in &answers {
            match *a {
                Answering::Eager { rel, input, n } => {
                    let args: Vec<T<'_>> = (0..n).map(x).collect();
                    b.rule().head(rel, &args).atom(input, &args).finish()?;
                }
                Answering::Function { rel, req, function, n } => {
                    let args: Vec<T<'_>> = (0..n).map(x).collect();
                    b.rule().head(rel, &args).atom(req, &[x(0)]).cmp(x(1), CmpOp::Eq, T::Call(function, vec![x(0)])).finish()?;
                }
                Answering::Children { rel, req, named, n } => {
                    let args: Vec<T<'_>> = (0..n).map(x).collect();
                    let mut by_node: Vec<T<'_>> = (0..n).map(x).collect();
                    by_node[2] = T::Var("c");
                    b.rule().head(rel, &args).atom(req, &[x(0)]).atom(named, &by_node).atom("node_text", &[T::Var("c"), x(2)]).finish()?;
                }
            }
        }
        for r in &wide.rules {
            r.write(v, b)?;
        }
        Ok(())
    }
}
