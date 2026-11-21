//! The static check (decision 0364 §5, R6): which recursive components cannot hold a row up by themselves.
//!
//! A component is proved when each of its relations has a *measure* column such that every recursive rule's own
//! atom is strictly on one side of its head: strictly above it in the tree, or strictly below it, or after it in
//! document order, or at the same node with no cycle of such steps, or (a self-recursion) one index step of
//! `@add(i, 1)` or `@sub(i, 1)`. Then no derivation cycle exists among its rows: a row's derivations read only rows
//! strictly before it along the measure, so its candidates can be rechecked once each in that order, and the
//! component keeps no levels.
//!
//! The port of `scripts/profiling/o125/recursion/wf.py`, over the program as the engine plans it. It is
//! conservative, as the script is: a step it cannot read (a helper between the two nodes it does not know) fails the
//! component, with the reason. It is more so in two places: only variables link one step to the next (the script
//! linked through `_` too), and a tree step with an index step in one component is refused.
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::analyses::Components;
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Reads};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// How a node relates to another through a rule's body: the same node, strictly below (`D`), below or the same
/// (`Dq`), strictly above (`U`), above or the same (`Uq`), after in document order (`S`), or no one way (`X`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
enum Step {
    Same,
    D,
    Dq,
    U,
    Uq,
    S,
    X,
}

impl Step {
    /// Its direction, and whether it may be the same node.
    fn dir(self) -> (char, bool) {
        match self {
            Step::D => ('D', false),
            Step::Dq => ('D', true),
            Step::U => ('U', false),
            Step::Uq => ('U', true),
            Step::S => ('S', false),
            Step::Same => ('=', true),
            Step::X => ('X', false),
        }
    }
    fn of(dir: char, loose: bool) -> Step {
        match (dir, loose) {
            ('D', false) => Step::D,
            ('D', true) => Step::Dq,
            ('U', false) => Step::U,
            ('U', true) => Step::Uq,
            ('S', _) => Step::S,
            _ => Step::X,
        }
    }
    /// A step `s` taken after `self`: one way, strict if either is.
    fn then(self, s: Step) -> Step {
        if self == Step::Same {
            return s;
        }
        if s == Step::Same {
            return self;
        }
        let ((d1, q1), (d2, q2)) = (self.dir(), s.dir());
        if d1 != d2 || d1 == 'S' {
            return Step::X;
        }
        Step::of(d1, q1 && q2)
    }
}

/// The tree atoms: `(relation, below, above)`: the atom's column `below` is strictly below its column `above`.
const BELOW: [(&str, usize, usize); 7] = [("child", 1, 0), ("field", 2, 0), ("has", 1, 0), ("within", 0, 1), ("inside", 0, 1), ("nearest", 0, 1), ("within_stop", 0, 1)];
/// Document order: `(relation, after, before)`.
const AFTER: [(&str, usize, usize); 2] = [("precedes", 1, 0), ("next_sibling", 1, 0)];

#[derive(Clone, Debug, PartialEq)]
enum Arg {
    Var(u32),
    Call(String, Vec<Arg>),
    Int(i64),
    Other,
}

impl Arg {
    fn var(&self) -> Option<u32> {
        if let Arg::Var(v) = self { Some(*v) } else { None }
    }
}

#[derive(Clone, Debug)]
struct Atom {
    rel: String,
    args: Vec<Arg>,
}

/// A rule as the check reads it: its head, its positive atoms, its equalities, and its `nearest` steps (`n` strictly
/// below `a`).
#[derive(Clone, Debug)]
struct Rule {
    head: Atom,
    atoms: Vec<Atom>,
    eqs: Vec<(Arg, Arg)>,
    nearest: Vec<(u32, u32)>,
}

fn arg(v: &Version, t: TermRef<'_>) -> Arg {
    match t {
        TermRef::Var(s) => Arg::Var(s.0),
        TermRef::Int(i) => Arg::Int(i),
        TermRef::Call(f, args) => Arg::Call(v.text(f).trim_start_matches('@').to_owned(), args.map(|a| arg(v, a)).collect()),
        _ => Arg::Other,
    }
}

fn rules_of(v: &Version) -> Vec<Rule> {
    v.rules()
        .map(|r| {
            let atom = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| Atom { rel: v.rel_name(a.rel()).to_owned(), args: a.args().map(|t| arg(v, t)).collect() };
            let mut rule = Rule { head: atom(r.head()), atoms: Vec::new(), eqs: Vec::new(), nearest: Vec::new() };
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) if !a.neg() => rule.atoms.push(atom(a)),
                    Lit::Cmp { l, op: CmpOp::Eq, r } => rule.eqs.push((arg(v, l), arg(v, r))),
                    Lit::Nearest { n, a, .. } => rule.nearest.push((n.0, a.0)),
                    _ => {}
                }
            }
            rule
        })
        .collect()
}

/// A helper's step from its column `i` to its column `j`, read off its own rules (`summarize`).
type Helpers = HashMap<(String, usize, usize), Step>;

/// How `y` relates to `x` through `rule`'s tree atoms, helpers and equalities: `None` where no one way is read.
fn order(rule: &Rule, helpers: &Helpers, x: u32, y: u32) -> Option<Step> {
    if x == y {
        return Some(Step::Same);
    }
    let mut e: HashMap<u32, Vec<(u32, Step)>> = HashMap::new();
    for (a, b) in &rule.eqs {
        if let (Some(a), Some(b)) = (a.var(), b.var()) {
            e.entry(a).or_default().push((b, Step::Same));
            e.entry(b).or_default().push((a, Step::Same));
        }
    }
    for &(n, a) in &rule.nearest {
        e.entry(a).or_default().push((n, Step::D));
        e.entry(n).or_default().push((a, Step::U));
    }
    for at in &rule.atoms {
        let col = |c: usize| at.args.get(c).and_then(Arg::var);
        // `nearest(n, a, set)` as the engine lowers it: `rr_nearest_<set>(n, a)`, `n` strictly below `a`.
        let below = BELOW.iter().find(|(r, _, _)| *r == at.rel).copied().or_else(|| at.rel.starts_with(crate::code_rule_dsl::logic::front::ast::NEAREST).then_some(("", 0, 1)));
        if let Some((_, b, u)) = below {
            if let (Some(b), Some(u)) = (col(b), col(u)) {
                e.entry(u).or_default().push((b, Step::D));
                e.entry(b).or_default().push((u, Step::U));
            }
        } else {
            for ((hr, i, j), k) in helpers {
                if *hr == at.rel
                    && let (Some(a), Some(b)) = (col(*i), col(*j))
                {
                    e.entry(a).or_default().push((b, *k));
                }
            }
        }
        if let Some(&(_, b, u)) = AFTER.iter().find(|(r, _, _)| *r == at.rel)
            && let (Some(b), Some(u)) = (col(b), col(u))
        {
            e.entry(u).or_default().push((b, Step::S));
        }
    }
    let mut seen: HashSet<(u32, Step)> = HashSet::from([(x, Step::Same)]);
    let mut stack = vec![(x, Step::Same)];
    while let Some((v, k)) = stack.pop() {
        for &(w, s) in e.get(&v).into_iter().flatten() {
            let nk = k.then(s);
            if nk == Step::X || !seen.insert((w, nk)) {
                continue;
            }
            stack.push((w, nk));
        }
    }
    let ks: BTreeSet<Step> = seen.iter().filter(|(v, k)| *v == y && *k != Step::Same).map(|(_, k)| *k).collect();
    match ks.len() {
        1 => ks.into_iter().next(),
        _ if seen.contains(&(y, Step::Same)) => Some(Step::Same),
        _ => None,
    }
}

/// Each relation with two node columns whose every rule relates them one way: its step from one to the other. A
/// self-recursion keeps the way of its other rules (not strictly, when one of those is the same node).
fn summarize(rules: &[Rule], types: &dyn Fn(&str) -> Vec<String>) -> Helpers {
    let mut by: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for r in rules {
        by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let mut helpers = Helpers::new();
    for _ in 0..3 {
        for (rel, rs) in &by {
            let cols: Vec<usize> = types(rel).iter().enumerate().filter(|(_, t)| *t == "node").map(|(i, _)| i).collect();
            for &i in &cols {
                for &j in &cols {
                    if i == j {
                        continue;
                    }
                    let (mut ks, mut rk, mut bad) = (Vec::new(), Vec::new(), false);
                    for r in rs {
                        let ha = &r.head.args;
                        if ha.len() <= i.max(j) {
                            bad = true;
                            break;
                        }
                        let recs: Vec<&Atom> = r.atoms.iter().filter(|a| a.rel == *rel).collect();
                        if !recs.is_empty() {
                            for ra in recs {
                                let (Some(hj), Some(rj)) = (ha[j].var(), ra.args.get(j).and_then(Arg::var)) else { bad = true; continue };
                                if hj != rj {
                                    bad = true;
                                    continue;
                                }
                                rk.push(ha[i].var().zip(ra.args.get(i).and_then(Arg::var)).and_then(|(x, y)| order(r, &helpers, x, y)));
                            }
                            continue;
                        }
                        ks.push(ha[i].var().zip(ha[j].var()).and_then(|(x, y)| order(r, &helpers, x, y)));
                    }
                    if bad || ks.is_empty() || ks.contains(&None) || rk.contains(&None) {
                        continue;
                    }
                    let mut ks: BTreeSet<Step> = ks.into_iter().flatten().collect();
                    let rk: Vec<Step> = rk.into_iter().flatten().collect();
                    let base: BTreeSet<char> = ks.iter().filter(|k| **k != Step::Same).map(|k| k.dir().0).collect();
                    if !rk.is_empty() {
                        if base.len() > 1 || !rk.iter().all(|k| *k != Step::Same && base.iter().next().is_none_or(|d| *d == k.dir().0)) {
                            continue;
                        }
                        let loose = ks.contains(&Step::Same);
                        ks.extend(rk.iter().map(|k| if loose { Step::of(k.dir().0, true) } else { *k }));
                    }
                    let dirs: BTreeSet<char> = ks.iter().filter(|k| **k != Step::Same).map(|k| k.dir().0).collect();
                    if dirs.len() != 1 || dirs.contains(&'S') {
                        continue;
                    }
                    let d = *dirs.iter().next().expect("one direction");
                    let strict = Step::of(d, false);
                    helpers.insert(((*rel).to_owned(), i, j), if ks == BTreeSet::from([strict]) { strict } else { Step::of(d, true) });
                }
            }
        }
    }
    helpers
}

/// A self-recursion's index step: the head's column `i` is the body atom's plus or minus one (`@add(j, 1)` in the
/// head, or `i = @add(j, 1)` in the body). `(climbs, column)`.
fn index_step(rule: &Rule, body: &Atom) -> Option<(bool, usize)> {
    let step = |t: &Arg, u: u32| match t {
        Arg::Call(f, a) if (f == "add" || f == "sub") && a.len() == 2 && a[0].var() == Some(u) && a[1] == Arg::Int(1) => Some(f == "add"),
        _ => None,
    };
    for (i, (t, u)) in rule.head.args.iter().zip(&body.args).enumerate() {
        let Some(u) = u.var() else { continue };
        if let Some(up) = step(t, u) {
            return Some((up, i));
        }
        if let Some(t) = t.var() {
            for (l, r) in &rule.eqs {
                let found = if l.var() == Some(t) { step(r, u) } else if r.var() == Some(t) { step(l, u) } else { None };
                if let Some(up) = found {
                    return Some((up, i));
                }
            }
        }
    }
    None
}

/// The order a proved component's candidates are rechecked in: by the node at each relation's measure column, in
/// preorder (`Ascending`: a row's derivations read rows above or before it), or the reverse; by an index column; or by
/// its relations' rank alone (every step at the same node).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// A row's derivations read rows at nodes above it: ancestors first.
    Down,
    /// They read rows at nodes below it: descendants first.
    Up,
    /// They read rows after it in document order: last first.
    Back,
    /// Its index climbs by one each step: the smallest first.
    Climbs,
    /// Its index falls by one each step: the largest first.
    Falls,
    /// Every step is at the same node: by rank alone.
    Same,
}

/// A proved component's measure: each relation's measure column (a node, or the index for `Climbs`/`Falls`) and its
/// rank among the relations a same-node step links (a row's derivations at its own node read relations of lower
/// rank), and the order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measure {
    pub order: Order,
    pub col: BTreeMap<String, usize>,
    pub rank: BTreeMap<String, u32>,
}

/// Whether component `comp` of `v` cannot hold a row up by itself, and its measure; or why not.
pub fn check(v: &Version, types: &dyn Fn(&str) -> Vec<String>, comp: &BTreeSet<String>) -> Result<Measure, String> {
    let rules = rules_of(v);
    let helpers = summarize(&rules, types);
    classify(&rules, &helpers, types, comp)
}

/// The static check of every recursive component of a program (preparation's, decision 0364 §5): an analysis of its
/// version, made once and shared. A part's run asks it of the program the part was split from, whose rules it reads
/// whole: a part reads a file-local helper as an input, or a join pushed into the chunks (`fl<n>_<rule>`, decision
/// 0051), whose rules the part does not hold.
#[derive(Debug, PartialEq)]
pub struct Proofs {
    /// Each recursive component, its relations sorted, and its verdict.
    comps: Vec<(BTreeSet<String>, Result<Measure, String>)>,
}

impl Proofs {
    /// The verdict on `scc`, which must be a component of the program: else not proved.
    pub fn of(&self, scc: &BTreeSet<String>) -> Result<Measure, String> {
        let first = scc.iter().next().ok_or("an empty component")?;
        match self.comps.iter().find(|(c, _)| c.contains(first)) {
            Some((c, v)) if c == scc => v.clone(),
            Some(_) => Err("not the program's component".into()),
            None => Err("not a recursive component of the program".into()),
        }
    }
    /// Each recursive component and its verdict.
    pub fn comps(&self) -> impl Iterator<Item = (&BTreeSet<String>, &Result<Measure, String>)> {
        self.comps.iter().map(|(c, v)| (c, v))
    }
    /// Proved and not, counted.
    pub fn counts(&self) -> (usize, usize) {
        let proved = self.comps.iter().filter(|(_, v)| v.is_ok()).count();
        (proved, self.comps.len() - proved)
    }
}

impl Analysis for Proofs {
    const NAME: &'static str = "well-founded components";
    const READS: Reads = Reads::RULES.or(Reads::DECLS).or(Reads::ABOUT);
    fn compute(cx: &Cx<'_>) -> Result<Proofs, String> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "well-founded components (decision 0364 §5)".into());
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(cx.v(), &d);
        let types = |r: &str| f.types(r).into_iter().map(str::to_owned).collect::<Vec<_>>();
        let rules = rules_of(cx.v());
        let helpers = summarize(&rules, &types);
        let comps = cx.get::<Components>()?;
        // A component is recursive when a rule of it reads it: found in one pass over the rules, by each relation's
        // component. A pass over every rule for each component took 0.16 s of the data layer's 6,842 rules (R7
        // against 0458); checking the recursive ones takes under 1 ms.
        let of: HashMap<&str, usize> = comps.0.iter().enumerate().flat_map(|(i, c)| c.iter().map(move |r| (r.as_str(), i))).collect();
        let mut recursive = vec![false; comps.0.len()];
        for r in &rules {
            if let Some(&c) = of.get(r.head.rel.as_str())
                && r.atoms.iter().any(|a| of.get(a.rel.as_str()) == Some(&c))
            {
                recursive[c] = true;
            }
        }
        let comps = comps.0.iter().zip(&recursive).filter(|(_, r)| **r).map(|(c, _)| (c.clone(), classify(&rules, &helpers, &types, c))).collect();
        Ok(Proofs { comps })
    }
}

fn classify(rules: &[Rule], helpers: &Helpers, types: &dyn Fn(&str) -> Vec<String>, comp: &BTreeSet<String>) -> Result<Measure, String> {
    let rs: Vec<&Rule> = rules.iter().filter(|r| comp.contains(&r.head.rel) && r.atoms.iter().any(|a| comp.contains(&a.rel))).collect();
    let rels: Vec<&String> = comp.iter().collect();
    let cols: Vec<Vec<usize>> = rels.iter().map(|r| types(r).iter().enumerate().filter(|(_, t)| *t == "node").map(|(i, _)| i).collect()).collect();
    if cols.iter().any(Vec::is_empty) {
        return Err("a relation with no node column".into());
    }
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    // Checked: a component of many relations with several node columns each overflowed the product before the bound.
    let Some(total) = cols.iter().try_fold(1usize, |n, c| n.checked_mul(c.len())).filter(|&n| n <= 4000) else {
        return Err("too many measures to try".into());
    };
    for n in 0..total {
        let (mut rest, mut m) = (n, BTreeMap::new());
        for (r, c) in rels.iter().zip(&cols) {
            m.insert((*r).clone(), c[rest % c.len()]);
            rest /= c.len();
        }
        let mut labels: BTreeSet<&'static str> = BTreeSet::new();
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        let mut eq: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut failed = None;
        'rules: for r in &rs {
            let h = &r.head.rel;
            for x in r.atoms.iter().filter(|a| comp.contains(&a.rel)) {
                let hx = r.head.args.get(m[h]).and_then(Arg::var);
                let bx = x.args.get(m[&x.rel]).and_then(Arg::var);
                let k = hx.zip(bx).and_then(|(a, b)| order(r, helpers, a, b));
                let label = match k {
                    Some(Step::Same) if x.rel == *h && let Some((up, i)) = index_step(r, x) => {
                        index.insert(h.clone(), i);
                        if up { "climbs" } else { "falls" }
                    }
                    Some(k @ (Step::Same | Step::Dq | Step::Uq)) => {
                        eq.entry(x.rel.clone()).or_default().insert(h.clone());
                        match k {
                            Step::Dq => "D",
                            Step::Uq => "U",
                            _ => continue,
                        }
                    }
                    Some(Step::D) => "D",
                    Some(Step::U) => "U",
                    Some(Step::S) => "S",
                    _ => {
                        let others: BTreeSet<&str> = r.atoms.iter().map(|a| a.rel.as_str()).filter(|a| !comp.contains(*a)).collect();
                        failed = Some(format!("an unknown step: {}", others.into_iter().collect::<Vec<_>>().join(", ")));
                        break 'rules;
                    }
                };
                labels.insert(label);
            }
        }
        if let Some(f) = failed {
            *why.entry(f).or_default() += 1;
            continue;
        }
        let tree: Vec<&&str> = labels.iter().filter(|l| !matches!(**l, "climbs" | "falls")).collect();
        let indexed = labels.len() - tree.len();
        if tree.len() > 1 || indexed > 1 {
            *why.entry("steps both ways".into()).or_default() += 1;
            continue;
        }
        if !tree.is_empty() && indexed > 0 {
            *why.entry("a tree step and an index step".into()).or_default() += 1;
            continue;
        }
        // No cycle of same-node steps: each relation ranked after those its rows at the same node read.
        let Some(rank) = ranked(comp, &eq) else {
            *why.entry("a cycle at one node".into()).or_default() += 1;
            continue;
        };
        let order = match labels.iter().next().copied() {
            None => Order::Same,
            Some("U") => Order::Down,
            Some("D") => Order::Up,
            Some("S") => Order::Back,
            Some("climbs") => Order::Climbs,
            Some(_) => Order::Falls,
        };
        if matches!(order, Order::Climbs | Order::Falls) {
            // The index is the measure: every relation of the component steps it (a self-recursion, one relation).
            if index.len() != comp.len() {
                *why.entry("an index step of some relations only".into()).or_default() += 1;
                continue;
            }
            m = index;
        }
        return Ok(Measure { order, col: m, rank });
    }
    Err(why.into_iter().max_by_key(|(_, n)| *n).map_or_else(|| "no measure".to_owned(), |(w, _)| w))
}

/// Each relation's rank: after every relation whose rows at the same node it reads (`eq`: read → readers). `None` on
/// a cycle, a relation reading itself at its own node included.
fn ranked(comp: &BTreeSet<String>, eq: &BTreeMap<String, BTreeSet<String>>) -> Option<BTreeMap<String, u32>> {
    let mut indeg: BTreeMap<&str, usize> = comp.iter().map(|r| (r.as_str(), 0)).collect();
    for (from, to) in eq {
        for t in to {
            if t == from {
                return None;
            }
            *indeg.entry(t.as_str()).or_default() += 1;
        }
    }
    let mut rank = BTreeMap::new();
    let mut ready: Vec<&str> = indeg.iter().filter(|(_, d)| **d == 0).map(|(r, _)| *r).collect();
    let mut next = 0u32;
    while let Some(r) = ready.pop() {
        rank.insert(r.to_owned(), next);
        next += 1;
        for t in eq.get(r).into_iter().flatten() {
            let d = indeg.get_mut(t.as_str()).expect("a relation of the component");
            *d -= 1;
            if *d == 0 {
                ready.push(t.as_str());
            }
        }
    }
    (rank.len() == indeg.len()).then_some(rank)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECLS: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end_: number)
.decl child(p: node, c: node, idx: number)
.decl edge(a: symbol, b: symbol)
.decl link(a: node, b: node)
"#;

    fn check_of(text: &str, comp: &[&str]) -> Result<Measure, String> {
        let p = crate::code_rule_dsl::logic::rr::parse_version(&format!("{DECLS}{text}")).expect("parses");
        let placed = super::super::body::Placed::whole(p.clone());
        let f = placed.fields().expect("fields");
        let comp: BTreeSet<String> = comp.iter().map(|s| (*s).to_owned()).collect();
        check(&placed.v, &|r: &str| f.types(r).into_iter().map(str::to_owned).collect(), &comp)
    }

    /// A walk down the tree is proved: each row is read from its parent's, ancestors first.
    #[test]
    fn a_walk_down_the_tree_cannot_cycle() {
        let m = check_of(
            ".decl top(a: node)\ntop(a) :- node(_, a, \"function_definition\", _, _).\n\
             .decl below(a: node, c: node)\nbelow(a, c) :- top(a), child(a, c, _).\nbelow(a, c) :- below(a, b), child(b, c, _).\n",
            &["below"],
        )
        .expect("proved");
        assert_eq!((m.order, m.col["below"]), (Order::Down, 1));
    }

    /// A step through a helper the check summarizes from its own rules (`desc`: column 1 below column 0).
    #[test]
    fn a_step_through_a_summarized_helper() {
        let m = check_of(
            ".decl desc(a: node, c: node)\ndesc(a, c) :- child(a, c, _).\ndesc(a, c) :- desc(a, b), child(b, c, _).\n\
             .decl seed(a: node)\nseed(a) :- node(_, a, \"module\", _, _).\n\
             .decl t(c: node)\nt(c) :- seed(c).\nt(c) :- t(a), desc(a, c).\n",
            &["t"],
        )
        .expect("proved");
        assert_eq!(m.order, Order::Down);
    }

    /// A graph that can cycle is not: its relation has no node column, or its step is a relation the check does not
    /// read as a tree step.
    #[test]
    fn graphs_that_can_cycle_are_not_proved() {
        let reach = check_of(".decl reach(a: symbol, b: symbol)\nreach(a, b) :- edge(a, b).\nreach(a, c) :- reach(a, b), edge(b, c).\n", &["reach"]);
        assert_eq!(reach, Err("a relation with no node column".into()));
        let linked = check_of(".decl r(a: node, b: node)\nr(a, b) :- link(a, b).\nr(a, c) :- r(a, b), link(b, c).\n", &["r"]);
        assert!(linked.unwrap_err().starts_with("an unknown step: link"));
    }

    /// Steps both ways, and a cycle at one node, fail.
    #[test]
    fn both_ways_or_a_cycle_at_one_node_fail() {
        let both = check_of(
            ".decl w(a: node)\nw(a) :- node(_, a, \"call\", _, _).\nw(c) :- w(a), child(a, c, _).\nw(a) :- w(c), child(a, c, _).\n",
            &["w"],
        );
        assert_eq!(both, Err("steps both ways".into()));
        let cycle = check_of(
            ".decl a(n: node)\n.decl b(n: node)\na(n) :- node(_, n, \"call\", _, _).\na(n) :- b(n).\nb(n) :- a(n).\n",
            &["a", "b"],
        );
        assert_eq!(cycle, Err("a cycle at one node".into()));
    }

    /// `nearest(n, a, set)` as the engine lowers it (`rr_nearest_<set>(n, a)`) is a step up: a walk out through
    /// scopes (calls.rerail's `walk`) is proved, nearer scopes first.
    #[test]
    fn a_walk_out_through_lowered_nearest_scopes() {
        let m = check_of(
            ".decl rr_nearest_fscope(n: node, a: node)\n.decl start(n: node, s: node)\nstart(n, s) :- rr_nearest_fscope(n, s).\n\
             .decl walk(n: node, s: node)\nwalk(n, s) :- start(n, s).\nwalk(n, p) :- walk(n, s), rr_nearest_fscope(s, p).\n",
            &["walk"],
        )
        .expect("proved");
        assert_eq!((m.order, m.col["walk"]), (Order::Up, 1));
    }

    /// The program's proofs answer for its components, and for no set that is not one.
    #[test]
    fn the_programs_proofs_answer_by_component() {
        let p = crate::code_rule_dsl::logic::rr::parse_version(&format!(
            "{DECLS}.decl below(a: node, c: node)\nbelow(a, c) :- child(a, c, _).\nbelow(a, c) :- below(a, b), child(b, c, _).\n\
             .decl reach(a: symbol, b: symbol)\nreach(a, b) :- edge(a, b).\nreach(a, c) :- reach(a, b), edge(b, c).\n.output below\n.output reach\n"
        ))
        .expect("parses");
        let placed = super::super::body::Placed::whole(p.clone());
        let proofs = crate::code_rule_dsl::logic::program_image::manager::analysis::<Proofs>(&placed.v).expect("proofs");
        let set = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<String>>();
        assert_eq!(proofs.counts(), (1, 1));
        assert_eq!(proofs.of(&set(&["below"])).map(|m| m.order), Ok(Order::Down));
        assert!(proofs.of(&set(&["reach"])).is_err());
        assert_eq!(proofs.of(&set(&["below", "reach"])), Err("not the program's component".into()));
    }

    /// Same-node steps without a cycle are ranked: a relation after the ones it reads at its node.
    #[test]
    fn same_node_steps_are_ranked() {
        let m = check_of(
            ".decl a(n: node)\n.decl b(n: node)\na(n) :- node(_, n, \"call\", _, _).\nb(n) :- a(n).\na(c) :- b(p), child(p, c, _).\n",
            &["a", "b"],
        )
        .expect("proved");
        assert_eq!(m.order, Order::Down);
        assert!(m.rank["a"] < m.rank["b"], "{m:?}");
    }
}
