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
//! - **an aggregate's value** is one for its group (the variables its atom shares with the rest of the rule): a key
//!   of the head holds where the group fixes the head's other columns, as `u = max i : …` keyed by its group.
//!
//! A relation of several rules, or a recursive one, keeps `local` where every rule gives it (a recursion's supposing
//! its component local, by induction on the rounds), and **a key** where every rule gives it and its rules are apart on
//! it (`apart`: no two rules give a row of the same key; the shape executors' "steps that give at most one row", the
//! design "Registering an executor for a join shape" §2). A recursion's key is proved by supposing it of its component
//! (0361: "recursion keeps a fact only when it is declared, or proved by the base case and the step"): each round's
//! rows then keep it, as each rule's row for a key is fixed by rows that keep theirs. Its other facts are declared
//! alone. A native's, a request's answer or the program's facts take their declared facts alone. `at_most_one`
//! answers, for a literal of a rule and the variables bound before it, whether it gives at most one row.
use rerail_blocks::vector::Vector;
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
    /// The engine's, for an input it makes from a kind set (`front::ast::made_from_a_kind_set`).
    Made,
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
        let mut out: BTreeMap<RelId, Vec<(LayoutFact, Origin)>> = about.layouts.iter().map(|e| (e.0, e.1.iter().map(|f| (<LayoutFact as rerail_section::Decode>::decode(f, &rerail_section::Names::NONE), Origin::Declared)).collect())).collect();
        // An input the transducer makes from a kind set (an axis's, `nearest`'s) has the engine's facts, where none
        // are declared (decision 0361 step 5): the planner sizes the rules over it with the program.
        for rel in v.inputs().filter_map(|a| v.rel_of(a)) {
            if out.contains_key(&rel) {
                continue;
            }
            if let Some(facts) = crate::code_rule_dsl::logic::front::ast::made_from_a_kind_set(v.rel_name(rel)) {
                out.insert(rel, facts.into_iter().map(|f| (f, Origin::Made)).collect());
            }
        }
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
                // (by induction on the rounds: the base rules read none of it). No other fact is carried. The
                // supposition is read beside the facts (`Known`), not written into a copy of them all.
                let local: BTreeMap<RelId, Vec<LayoutFact>> = scc.iter().map(|r| (*r, vec![LayoutFact::Local])).collect();
                let supposed = Known { facts: &out, supposed: &local };
                let is_local = !rules.is_empty() && rules.iter().all(|r| infer_rule(v, *r, &supposed, &[]).contains(&LayoutFact::Local));
                let nothing = BTreeMap::new();
                let keys = component_keys(v, scc, &out, if is_local { &local } else { &nothing });
                for (rel, keys) in keys {
                    let facts = out.entry(rel).or_default();
                    for k in keys {
                        let k = LayoutFact::Key(k);
                        if !facts.iter().any(|(f, _)| *f == k) {
                            facts.push((k, Origin::Rules));
                        }
                    }
                }
                if is_local {
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
            let inferred = infer_rule(v, rule, &Known { facts: &out, supposed: &BTreeMap::new() }, &[]);
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
                    Origin::Made => format!("{} (the engine's)", f.show(&col)),
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

/// The facts known of each relation, and the relations supposed local besides: a recursive component's or a union's,
/// supposed while its rules are inferred (`Layouts::infer`).
struct Known<'a> {
    facts: &'a BTreeMap<RelId, Vec<(LayoutFact, Origin)>>,
    /// The facts supposed of the component's relations, besides their known ones.
    supposed: &'a BTreeMap<RelId, Vec<LayoutFact>>,
}

impl Known<'_> {
    /// `rel`'s facts: its known ones, and those supposed of it.
    fn of(&self, rel: RelId) -> impl Iterator<Item = &LayoutFact> + '_ {
        self.facts.get(&rel).into_iter().flatten().map(|(f, _)| f).chain(self.supposed.get(&rel).into_iter().flatten())
    }
}

/// The keys of a union's or a recursion's relations (`scc`): a key each rule of a relation gives, its rules apart on it
/// (`apart`), with every relation of the component supposed to keep the keys still standing, until none falls (the
/// greatest such set: by induction on the rounds, each round's rows keep them). The candidates are the keys a rule gives
/// with nothing supposed, or with its relation's other rules' keys supposed of it. `local`: the component's `local`, as
/// proved, supposed beside them.
fn component_keys(v: &Version, scc: &[RelId], out: &BTreeMap<RelId, Vec<(LayoutFact, Origin)>>, local: &BTreeMap<RelId, Vec<LayoutFact>>) -> BTreeMap<RelId, Vec<Vector<usize>>> {
    let with = |keys: &BTreeMap<RelId, Vec<Vector<usize>>>| -> BTreeMap<RelId, Vec<LayoutFact>> {
        scc.iter().map(|r| (*r, keys.get(r).into_iter().flatten().map(|k| LayoutFact::Key(k.clone())).chain(local.get(r).into_iter().flatten().cloned()).collect())).collect()
    };
    let declared = |rel: RelId, k: &Vector<usize>| out.get(&rel).is_some_and(|fs| fs.iter().any(|(f, _)| matches!(f, LayoutFact::Key(d) if d == k)));
    let key_of = |f: LayoutFact| if let LayoutFact::Key(k) = f { Some(k) } else { None };
    // The candidates: every key a rule of the relation gives, nothing supposed but `local`.
    let none = Known { facts: out, supposed: local };
    let mut keys: BTreeMap<RelId, Vec<Vector<usize>>> = BTreeMap::new();
    for &rel in scc {
        let mut ks: Vec<Vector<usize>> = Vec::new();
        for r in v.heads(rel) {
            for k in infer_rule(v, r, &none, &[]).into_iter().filter_map(key_of) {
                if !ks.contains(&k) && !declared(rel, &k) {
                    ks.push(k);
                }
            }
        }
        if !ks.is_empty() {
            keys.insert(rel, ks);
        }
    }
    loop {
        let supposed = with(&keys);
        let known = Known { facts: out, supposed: &supposed };
        let mut fell = false;
        for &rel in scc {
            let rules: Vec<RuleRef<'_>> = v.heads(rel).collect();
            let Some(ks) = keys.get_mut(&rel) else { continue };
            ks.retain(|k| {
                let given = rules.iter().all(|r| infer_rule(v, *r, &known, std::slice::from_ref(k)).contains(&LayoutFact::Key(k.clone())));
                let kept = given && (0..rules.len()).all(|i| (i + 1..rules.len()).all(|j| apart(rules[i], rules[j], k, &known)));
                fell |= !kept;
                kept
            });
        }
        keys.retain(|_, ks| !ks.is_empty());
        if !fell {
            return keys;
        }
    }
}

/// A term of either of two rules (`false` the first's), or a constant, for `apart`'s classes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Node {
    Var(bool, Var),
    Int(i64),
    Str(super::AtomId),
}

impl Node {
    fn of(side: bool, t: TermRef<'_>) -> Option<Node> {
        match t {
            TermRef::Var(s) => Some(Node::Var(side, s)),
            TermRef::Int(i) => Some(Node::Int(i)),
            TermRef::Str(s) => Some(Node::Str(s)),
            _ => None,
        }
    }
}

/// `apart`'s classes of terms known equal: a union-find over the terms it has met.
#[derive(Default)]
struct Classes {
    ids: HashMap<Node, usize>,
    up: Vec<usize>,
}

impl Classes {
    fn id(&mut self, n: Node) -> usize {
        let next = self.up.len();
        let i = *self.ids.entry(n).or_insert(next);
        if i == next {
            self.up.push(next);
        }
        i
    }
    fn root(&mut self, mut x: usize) -> usize {
        while self.up[x] != x {
            self.up[x] = self.up[self.up[x]];
            x = self.up[x];
        }
        x
    }
    /// Equates `x` and `y`; whether they were apart.
    fn merge(&mut self, x: Node, y: Node) -> bool {
        let (i, j) = (self.id(x), self.id(y));
        let (ri, rj) = (self.root(i), self.root(j));
        (ri != rj).then(|| self.up[ri] = rj).is_some()
    }
    /// Whether terms `x` and `y` are known equal.
    fn same(&mut self, x: Option<Node>, y: Option<Node>) -> bool {
        let (Some(x), Some(y)) = (x, y) else { return false };
        x == y || match (self.ids.get(&x).copied(), self.ids.get(&y).copied()) {
            (Some(i), Some(j)) => self.root(i) == self.root(j),
            _ => false,
        }
    }
    /// Whether a class holds two constants.
    fn clash(&mut self) -> bool {
        let consts: Vec<(Node, usize)> = self.ids.iter().filter(|(n, _)| matches!(n, Node::Int(_) | Node::Str(_))).map(|(n, i)| (*n, *i)).collect();
        let mut seen: HashMap<usize, Node> = HashMap::new();
        consts.into_iter().any(|(n, i)| {
            let r = self.root(i);
            seen.insert(r, n).is_some_and(|m| m != n)
        })
    }
}

/// A rule's body, as `apart` reads it.
struct Body<'v> {
    pos: Vec<(RelId, Vec<TermRef<'v>>)>,
    neg: Vec<(RelId, Vec<TermRef<'v>>)>,
    nearest: Vec<(Var, Var, super::AtomId)>,
    eqs: Vec<(TermRef<'v>, TermRef<'v>)>,
}

impl<'v> Body<'v> {
    fn of(r: RuleRef<'v>) -> Body<'v> {
        let mut b = Body { pos: Vec::new(), neg: Vec::new(), nearest: Vec::new(), eqs: Vec::new() };
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) if a.neg() => b.neg.push((a.rel(), a.args().collect())),
                Lit::Atom(a) => b.pos.push((a.rel(), a.args().collect())),
                Lit::Cmp { l, op: CmpOp::Eq, r } => b.eqs.push((l, r)),
                Lit::Nearest { n, a, kindset } => b.nearest.push((n, a, kindset)),
                _ => {}
            }
        }
        b
    }
}

/// Whether rules `a` and `b` of one relation are apart on its key `key`: no row of one has the key of a row of the
/// other. Their head's key columns are equated, and with them every term a key fixes in both (an atom of the same
/// relation in each, keyed by terms already equal, has its other columns equal; `nearest` of one kind set from equal
/// nodes, its ancestors). They are apart where a class then holds two constants (a driver of another kind), or one
/// rule's positive atom is the other's negated (a test one passes and the other fails).
fn apart(a: RuleRef<'_>, b: RuleRef<'_>, key: &[usize], known: &Known<'_>) -> bool {
    let mut cls = Classes::default();
    let (ha, hb): (Vec<TermRef<'_>>, Vec<TermRef<'_>>) = (a.head().args().collect(), b.head().args().collect());
    for &c in key {
        let (Some(x), Some(y)) = (ha.get(c).and_then(|t| Node::of(false, *t)), hb.get(c).and_then(|t| Node::of(true, *t))) else { return false };
        cls.merge(x, y);
    }
    let (ba, bb) = (Body::of(a), Body::of(b));
    for (side, body) in [(false, &ba), (true, &bb)] {
        for &(l, r) in &body.eqs {
            if let (Some(x), Some(y)) = (Node::of(side, l), Node::of(side, r)) {
                cls.merge(x, y);
            }
        }
    }
    // Along the keys of atoms of one relation in both, and along `nearest`, until no class grows.
    loop {
        let mut grew = false;
        for (rel, xa) in &ba.pos {
            for (_, xb) in bb.pos.iter().filter(|(r, _)| r == rel) {
                for f in known.of(*rel) {
                    let LayoutFact::Key(cols) = f else { continue };
                    let keyed = cols.iter().all(|&c| {
                        let (x, y) = (xa.get(c).and_then(|t| Node::of(false, *t)), xb.get(c).and_then(|t| Node::of(true, *t)));
                        cls.same(x, y)
                    });
                    if !keyed {
                        continue;
                    }
                    for (c, (ta, tb)) in xa.iter().zip(xb).enumerate() {
                        if let (false, Some(x), Some(y)) = (cols.contains(&c), Node::of(false, *ta), Node::of(true, *tb)) {
                            grew |= cls.merge(x, y);
                        }
                    }
                }
            }
        }
        for &(na, aa, ka) in &ba.nearest {
            for &(nb, ab, _) in bb.nearest.iter().filter(|n| n.2 == ka) {
                if cls.same(Some(Node::Var(false, na)), Some(Node::Var(true, nb))) {
                    grew |= cls.merge(Node::Var(false, aa), Node::Var(true, ab));
                }
            }
        }
        if !grew {
            break;
        }
    }
    if cls.clash() {
        return true;
    }
    // One's positive atom the other's negated: each column of the negated a wildcard, or equal to the positive's.
    let mut test = |pos: &[(RelId, Vec<TermRef<'_>>)], ps: bool, neg: &[(RelId, Vec<TermRef<'_>>)], ns: bool| {
        neg.iter().any(|(rel, xn)| {
            pos.iter().filter(|(r, xp)| r == rel && xp.len() == xn.len()).any(|(_, xp)| {
                xn.iter().zip(xp).all(|(tn, tp)| matches!(tn, TermRef::Wild) || cls.same(Node::of(ns, *tn), Node::of(ps, *tp)))
            })
        })
    };
    test(&ba.pos, false, &bb.neg, true) || test(&bb.pos, true, &ba.neg, false)
}

impl Layouts {
    /// Whether literal `lit` gives at most one row for each binding of the variables bound before it (`bound`): the
    /// step the shape executors chain (the design "Registering an executor for a join shape" §2). An atom does where
    /// a key of its relation is bound (constants, or variables of `bound`), or every column is (a test); a negated
    /// atom, a comparison and an aggregate (one value for its group) always; `nearest` where its node is bound. A
    /// matcher's captures are not.
    pub fn at_most_one(&self, lit: super::handles::LitRef<'_>, bound: &BTreeSet<Var>) -> bool {
        match lit.view() {
            Lit::Atom(a) if a.neg() => true,
            Lit::Atom(a) => {
                let args: Vec<TermRef<'_>> = a.args().collect();
                let is_bound = |c: usize| match args.get(c) {
                    Some(TermRef::Var(s)) => bound.contains(s),
                    Some(TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_)) => true,
                    _ => false,
                };
                (0..args.len()).all(is_bound) || self.of(a.rel()).any(|f| matches!(f, LayoutFact::Key(cols) if cols.iter().all(|&c| is_bound(c))))
            }
            Lit::Cmp { .. } | Lit::Agg { .. } => true,
            Lit::Nearest { n, .. } => bound.contains(&n),
            Lit::Ast { .. } => false,
        }
    }
}

/// What one rule gives its head, from the facts known of the relations its body reads (`known`). `also`: head columns
/// tried as a key besides the body's own (an atom's key, an aggregate's group): another rule's, or one supposed.
fn infer_rule(v: &Version, r: RuleRef<'_>, known: &Known<'_>, also: &[Vector<usize>]) -> Vec<LayoutFact> {
    let facts_of = |rel: RelId| known.of(rel);
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
    // The variables outside each aggregate's atom: the head's and every other literal's. An aggregate's group is the
    // variables its atom shares with them.
    let outer = |skip: u32| -> BTreeSet<Var> {
        let mut vs = head_vars.clone();
        for l in r.body() {
            if l.node() == skip {
                continue;
            }
            match l.view() {
                Lit::Atom(a) => a.args().for_each(|t| vars_of(t, &mut vs)),
                Lit::Cmp { l, r, .. } => {
                    vars_of(l, &mut vs);
                    vars_of(r, &mut vs);
                }
                Lit::Agg { var, op, atom, .. } => {
                    vs.insert(var);
                    vs.extend(op.bound());
                    let _ = atom;
                }
                Lit::Nearest { n, a, .. } => {
                    vs.extend([n, a]);
                }
                Lit::Ast { document, node, captures, .. } => {
                    vs.extend([document, node]);
                    vs.extend(captures.map(|(_, s)| s));
                }
            }
        }
        vs
    };
    let mut groups: Vec<BTreeSet<Var>> = Vec::new();
    let mut aggregate = false;
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
            // An aggregate's value is one for its group: the group fixes it.
            Lit::Agg { var, op, atom, .. } => {
                only_atom = false;
                aggregate = true;
                let mut inner = BTreeSet::new();
                atom.args().for_each(|t| vars_of(t, &mut inner));
                let mut group: BTreeSet<Var> = inner.intersection(&outer(l.node())).copied().collect();
                group.extend(op.bound());
                group.remove(&var);
                deps.push((group.clone(), BTreeSet::from([var])));
                groups.push(group);
            }
            // A matcher makes values no fact describes.
            Lit::Ast { .. } => return Vec::new(),
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
    let places = |vs: &[Var]| -> Option<rerail_blocks::vector::Vector<usize>> { vs.iter().map(|s| place.get(s).copied()).collect() };
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
    // An aggregate's group, and the columns tried besides (`also`), keep a key where they fix the head's row.
    let tried = also.iter().filter_map(|cols| {
        let mut ks = BTreeSet::new();
        for &c in cols.iter() {
            match head.get(c)? {
                TermRef::Var(s) => {
                    ks.insert(*s);
                }
                TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => {}
                _ => return None,
            }
        }
        Some(ks)
    });
    for ks in groups.iter().cloned().chain(tried.collect::<Vec<_>>()) {
        if fixes(&ks)
            && ks.len() < head.len()
            && let Some(mut at) = places(&ks.iter().copied().collect::<Vec<_>>())
        {
            at.sort_unstable();
            add(LayoutFact::Key(at));
        }
    }
    if !aggregate && local(v, r.head().rel(), &atoms, &nearest, &head, known) {
        add(LayoutFact::Local);
    }
    out
}

/// Whether a rule's rows are local: every atom is, every node the head holds is an atom's, and the atoms that hold
/// nodes are one group, joined by their shared nodes or by `nearest`.
fn local(v: &Version, head_rel: RelId, atoms: &[(RelId, Vec<TermRef<'_>>)], nearest: &[(Var, Var)], head: &[TermRef<'_>], known: &Known<'_>) -> bool {
    let node_col = |rel: RelId, c: usize| v.decl_of(v.rel_name(rel)).and_then(|d| d.cols().nth(c)).is_some_and(|(_, t)| matches!(v.text(t), "node" | "loc"));
    // Local: stated or inferred, or by its shape: a relation of one node column at most crosses no documents.
    let is_local = |rel: RelId| {
        known.of(rel).any(|f| *f == LayoutFact::Local)
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
    let with: Vector<usize> = (0..atoms.len()).filter(|&k| !nodes[k].is_empty()).collect();
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

