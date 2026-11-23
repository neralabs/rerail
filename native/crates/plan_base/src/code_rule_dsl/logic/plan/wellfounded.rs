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
pub enum Step {
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

/// A helper's step from its column `i` to its column `j`, read off its own rules (`summarize`), by the helper: an atom's
/// steps are one lookup of its relation. Kept as one map of every (relation, i, j), each atom of each rule `order` read
/// scanned all of them: 83,170 rules on main's Python pack, past 31 s.
type Helpers = HashMap<String, BTreeMap<(usize, usize), Step>>;

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
            for (&(i, j), k) in helpers.get(&at.rel).into_iter().flatten() {
                if let (Some(a), Some(b)) = (col(i), col(j)) {
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
/// self-recursion keeps the way of its other rules (not strictly, when one of those is the same node). Only the
/// relations `roots` read through their rules, transitively: a helper's step is read only where an atom of it is, and
/// a relation's step reads only those of the relations its rules read, so the steps are the same as over every rule.
fn summarize<'a>(rules: &'a [Rule], types: &dyn Fn(&str) -> Vec<String>, roots: &[&str]) -> Helpers {
    let mut heads: HashMap<&'a str, Vec<&'a Rule>> = HashMap::new();
    for r in rules {
        heads.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let mut keep: BTreeSet<&'a str> = roots.iter().filter_map(|r| heads.get_key_value(*r).map(|(k, _)| *k)).collect();
    let mut stack: Vec<&'a str> = keep.iter().copied().collect();
    while let Some(rel) = stack.pop() {
        for r in heads.get(rel).into_iter().flatten() {
            for a in &r.atoms {
                if let Some((k, _)) = heads.get_key_value(a.rel.as_str())
                    && keep.insert(k)
                {
                    stack.push(k);
                }
            }
        }
    }
    let by: Vec<(&str, Vec<usize>, &Vec<&Rule>)> = keep
        .iter()
        .map(|rel| (*rel, types(rel).iter().enumerate().filter(|(_, t)| *t == "node").map(|(i, _)| i).collect(), &heads[rel]))
        .collect();
    let mut helpers = Helpers::new();
    for _ in 0..3 {
        for (rel, cols, rs) in &by {
            for &i in cols {
                for &j in cols {
                    if i == j {
                        continue;
                    }
                    let (mut ks, mut rk, mut bad) = (Vec::new(), Vec::new(), false);
                    for r in rs.iter() {
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
                    helpers.entry((*rel).to_owned()).or_default().insert((i, j), if ks == BTreeSet::from([strict]) { strict } else { Step::of(d, true) });
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
    let roots: Vec<&str> = comp.iter().map(String::as_str).collect();
    let helpers = summarize(&rules, types, &roots);
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
        let roots: Vec<&str> = comps.0.iter().zip(&recursive).filter(|(_, r)| **r).flat_map(|(c, _)| c.iter().map(String::as_str)).collect();
        let helpers = summarize(&rules, &types, &roots);
        let comps = comps.0.iter().zip(&recursive).filter(|(_, r)| **r).map(|(c, _)| (c.clone(), classify(&rules, &helpers, &types, c))).collect();
        Ok(Proofs { comps })
    }
    /// Kept by the version's identity (`plan::kept_analyses`, t33 K4): a component's row is its place, its verdict
    /// (an order's code, or -1 and why not) and its relations; a `col` row a measured relation's column and rank.
    fn rows(&self, _v: &Version) -> Option<Vec<crate::code_rule_dsl::logic::program_image::manager::Row>> {
        use crate::code_rule_dsl::logic::front::Value;
        let s = |x: &str| Value::Str(x.into());
        let mut out = Vec::new();
        for (k, (comp, verdict)) in self.comps.iter().enumerate() {
            let (code, why) = match verdict {
                Ok(m) => (ORDERS.iter().position(|o| *o == m.order).expect("an order") as i64, String::new()),
                Err(e) => (-1, e.clone()),
            };
            out.push([s("comp"), Value::Int(k as i64), Value::Int(code), s(&why)].into_iter().chain(comp.iter().map(|r| s(r))).collect());
            if let Ok(m) = verdict {
                for (rel, col) in &m.col {
                    out.push(vec![s("col"), Value::Int(k as i64), s(rel), Value::Int(*col as i64), Value::Int(m.rank.get(rel).map_or(-1, |r| i64::from(*r)))]);
                }
                for (rel, rank) in m.rank.iter().filter(|(r, _)| !m.col.contains_key(*r)) {
                    out.push(vec![s("col"), Value::Int(k as i64), s(rel), Value::Int(-1), Value::Int(i64::from(*rank))]);
                }
            }
        }
        Some(out)
    }
    fn of_rows(_v: &Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<Proofs> {
        use crate::code_rule_dsl::logic::front::Value;
        let text = |v: &Value| match v {
            Value::Str(x) => Some(x.to_string()),
            _ => None,
        };
        let int = |v: &Value| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        };
        let mut comps: Vec<(BTreeSet<String>, Result<Measure, String>)> = Vec::new();
        for r in rows {
            match text(r.first()?)?.as_str() {
                "comp" => {
                    if usize::try_from(int(r.get(1)?)?).ok()? != comps.len() {
                        return None;
                    }
                    let code = int(r.get(2)?)?;
                    let verdict = match code {
                        -1 => Err(text(r.get(3)?)?),
                        c => Ok(Measure { order: *ORDERS.get(usize::try_from(c).ok()?)?, col: BTreeMap::new(), rank: BTreeMap::new() }),
                    };
                    comps.push((r.get(4..)?.iter().map(text).collect::<Option<_>>()?, verdict));
                }
                "col" => {
                    let k = usize::try_from(int(r.get(1)?)?).ok()?;
                    let Ok(m) = &mut comps.get_mut(k)?.1 else { return None };
                    let rel = text(r.get(2)?)?;
                    let (col, rank) = (int(r.get(3)?)?, int(r.get(4)?)?);
                    if col >= 0 {
                        m.col.insert(rel.clone(), usize::try_from(col).ok()?);
                    }
                    if rank >= 0 {
                        m.rank.insert(rel, u32::try_from(rank).ok()?);
                    }
                }
                _ => return None,
            }
        }
        Some(Proofs { comps })
    }
}

/// The orders, by their kept code (`Proofs::rows`).
const ORDERS: [Order; 6] = [Order::Down, Order::Up, Order::Back, Order::Climbs, Order::Falls, Order::Same];

fn classify(rules: &[Rule], helpers: &Helpers, types: &dyn Fn(&str) -> Vec<String>, comp: &BTreeSet<String>) -> Result<Measure, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("a recursive component classified ({} relations)", comp.len()));
    let rs: Vec<&Rule> = rules.iter().filter(|r| comp.contains(&r.head.rel) && r.atoms.iter().any(|a| comp.contains(&a.rel))).collect();
    let rels: Vec<&String> = comp.iter().collect();
    let cols: Vec<Vec<usize>> = rels.iter().map(|r| types(r).iter().enumerate().filter(|(_, t)| *t == "node").map(|(i, _)| i).collect()).collect();
    if cols.iter().any(Vec::is_empty) {
        return Err("a relation with no node column".into());
    }
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    // The measures to try: one node column of each relation, every choice. Stopped at the bound as it is multiplied:
    // a component of many relations (python/calls with the typing run, about 2^64 choices) overflowed the product.
    const MEASURES: usize = 4000;
    let Some(total) = cols.iter().try_fold(1usize, |n, c| n.checked_mul(c.len()).filter(|&n| n <= MEASURES)) else {
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus)
