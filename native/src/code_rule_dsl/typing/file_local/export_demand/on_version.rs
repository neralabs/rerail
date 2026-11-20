//! The export demand's cut (`export_demand`'s module comment), on the version (decision 0281's path, step 5),
//! reading the split from the version (`FileLocal`). Variables are read by name, so every set and search runs in a
//! fixed order and makes its rules; each round's cut is a version of its own, split again to check it stays
//! file-local.

use super::super::FileLocal;
use super::{MAX_KEYS, ROUNDS, on};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, T};
use crate::code_rule_dsl::logic::program_image::manager::{self, Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, CmpOp, Lit as L, LitRef, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

type Key = (String, usize);

/// Each export cut to its cross-file readers' demand: a line per export cut, and why the pass made none.
#[derive(Default)]
pub struct DemandExports {
    pub lines: Vec<String>,
}

fn name(v: &Version, rule: RuleRef<'_>, s: Slot) -> String {
    v.text(rule.var(s)).to_owned()
}

fn term_vars(v: &Version, rule: RuleRef<'_>, t: TermRef<'_>, out: &mut BTreeSet<String>) {
    match t {
        TermRef::Var(s) => {
            out.insert(name(v, rule, s));
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(v, rule, a, out)),
        _ => {}
    }
}

fn atom_vars(v: &Version, rule: RuleRef<'_>, a: AtomRef<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    a.args().for_each(|t| term_vars(v, rule, t, &mut out));
    out
}

/// A term's text, to tell equal literals apart as the tree's equality does.
fn term_text(v: &Version, rule: RuleRef<'_>, t: TermRef<'_>) -> String {
    match t {
        TermRef::Var(s) => name(v, rule, s),
        TermRef::Wild => "_".into(),
        TermRef::Top => "⊤".into(),
        TermRef::Int(n) => n.to_string(),
        TermRef::Str(a) => format!("{:?}", v.text(a)),
        // A branded atom (decision 0270) is equal to another only as the same atom.
        TermRef::Branded(a) => format!("#{}", a.value()),
        TermRef::Call(f, args) => format!("@{}({})", v.text(f), args.map(|a| term_text(v, rule, a)).collect::<Vec<_>>().join(",")),
    }
}

fn is_const(t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_))
}

fn find(alias: &BTreeMap<String, String>, v: &str) -> String {
    let mut v = v.to_owned();
    while let Some(n) = alias.get(&v) {
        v = n.clone();
    }
    v
}

/// A superset rule before the supersets that are `top` are known: of `rule`, its head variable, the literals kept, the
/// supersets it filters by, `(key, variable)`, and the variables the kept atoms and comparisons bind, without the
/// filters.
struct Shape<'v> {
    rule: RuleRef<'v>,
    head: String,
    kept: Vec<LitRef<'v>>,
    filters: Vec<(Key, String)>,
    bound: BTreeSet<String>,
}

#[derive(Clone, Copy)]
enum Const {
    Int(i64),
    Str(AtomId),
    /// A branded atom (decision 0270), a constant as the tree's `Term::Branded` is.
    Branded(crate::code_rule_dsl::logic::front::Branded),
}

/// A reader's filter on an export it reads: a file-local atom on the variable at its column `at` (its constants kept),
/// read through its distinct keys there (`key_name`), or a superset.
enum Filter {
    Atom { rel: String, width: usize, at: usize, consts: Vec<(usize, Const)>, v: String },
    Key(Key, String),
}

struct Pass<'a> {
    v: &'a Version,
    f: Fields<'a>,
    local: &'a BTreeSet<String>,
    locations: &'a BTreeMap<String, Vec<usize>>,
    same_doc: &'a BTreeMap<String, Vec<usize>>,
    rules_of: HashMap<&'a str, Vec<RuleRef<'a>>>,
    shapes: BTreeMap<Key, Option<Vec<Shape<'a>>>>,
}

impl<'a> Pass<'a> {
    /// A relation the chunk holds by document: file-local or a base input of the chunk.
    fn usable(&self, rel: &str) -> bool {
        (self.local.contains(rel) || self.f.is_input(rel)) && !matches!(rel, "reference_edge" | "reference_open") && self.same_doc.contains_key(rel)
    }
    fn same(&self, rel: &str, at: usize) -> bool {
        self.same_doc.get(rel).is_some_and(|c| c.contains(&at))
    }
    fn located(&self, rel: &str, at: usize) -> bool {
        self.locations.get(rel).is_some_and(|c| c.contains(&at))
    }
    fn rel(&self, a: AtomRef<'_>) -> &'a str {
        self.v.rel_name(a.rel())
    }

    /// The superset of `key`'s column, explored with every key it reads: `None` when it is `top` from the start.
    fn explore(&mut self, key: Key) {
        let mut work = vec![key];
        while let Some(key) = work.pop() {
            if self.shapes.contains_key(&key) {
                continue;
            }
            if self.shapes.len() >= MAX_KEYS {
                self.shapes.insert(key, None);
                continue;
            }
            let shapes = self.shapes_of(&key);
            if let Some(shapes) = &shapes {
                work.extend(shapes.iter().flat_map(|s| s.filters.iter().map(|(k, _)| k.clone())));
            }
            self.shapes.insert(key, shapes);
        }
    }

    fn shapes_of(&self, (rel, at): &Key) -> Option<Vec<Shape<'a>>> {
        if self.f.native(rel).is_some() || self.f.lattice(rel) || self.local.contains(rel) {
            return None;
        }
        let rules = self.rules_of.get(rel.as_str())?;
        rules.iter().map(|r| self.shape(*r, *at)).collect()
    }

    /// One rule of a cross-file relation, cut to the literals in the document of its column `at`.
    fn shape(&self, rule: RuleRef<'a>, at: usize) -> Option<Shape<'a>> {
        let v = self.v;
        let Some(TermRef::Var(h)) = rule.head().args().nth(at) else { return None };
        let head = name(v, rule, h);
        let body: Vec<LitRef<'a>> = rule.body().collect();
        let mut alias: BTreeMap<String, String> = BTreeMap::new();
        for l in &body {
            if let L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
                let (a, b) = (find(&alias, &name(v, rule, a)), find(&alias, &name(v, rule, b)));
                if a != b {
                    alias.insert(a, b);
                }
            }
        }
        let mut reach: BTreeSet<String> = BTreeSet::from([find(&alias, &head)]);
        let mut keep = vec![false; body.len()];
        loop {
            let mut grew = false;
            for (i, l) in body.iter().enumerate() {
                if keep[i] {
                    continue;
                }
                let joined: Vec<String> = match l.view() {
                    L::Atom(a) if !a.neg() && self.usable(self.rel(a)) => a
                        .args()
                        .enumerate()
                        .filter(|(j, _)| self.same(self.rel(a), *j))
                        .filter_map(|(_, t)| t.var().map(|s| find(&alias, &name(v, rule, s))))
                        .collect(),
                    L::Nearest { n, a, .. } => vec![find(&alias, &name(v, rule, n)), find(&alias, &name(v, rule, a))],
                    _ => continue,
                };
                if joined.iter().any(|x| reach.contains(x)) {
                    keep[i] = true;
                    reach.extend(joined);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut kept: Vec<LitRef<'a>> = Vec::new();
        let mut bound: BTreeSet<String> = BTreeSet::new();
        for (i, l) in body.iter().enumerate() {
            if keep[i] && let L::Atom(a) = l.view() {
                bound.extend(atom_vars(v, rule, a));
                kept.push(*l);
            }
        }
        for (i, l) in body.iter().enumerate() {
            if keep[i] && let L::Nearest { n, a, .. } = l.view() && bound.contains(&name(v, rule, n)) {
                bound.insert(name(v, rule, a));
                kept.push(*l);
            }
        }
        let mut filters: Vec<(Key, String)> = Vec::new();
        for l in &body {
            if let L::Atom(a) = l.view() && !a.neg() && !self.usable(self.rel(a)) {
                for (j, t) in a.args().enumerate() {
                    if let TermRef::Var(s) = t && reach.contains(&find(&alias, &name(v, rule, s))) && self.located(self.rel(a), j) {
                        filters.push(((self.rel(a).to_owned(), j), name(v, rule, s)));
                    }
                }
            }
        }
        // Comparisons over bound variables (an equality binds its other side), kept in order, each text once.
        let mut cmps: Vec<LitRef<'a>> = Vec::new();
        let mut cmp_texts: Vec<String> = Vec::new();
        let filtered: BTreeSet<String> = filters.iter().map(|(_, x)| x.clone()).collect();
        loop {
            let mut grew = false;
            for l in &body {
                let L::Cmp { l: a, op, r: b } = l.view() else { continue };
                let text = format!("{} {} {}", term_text(v, rule, a), op.as_str(), term_text(v, rule, b));
                if cmp_texts.contains(&text) {
                    continue;
                }
                let vars = |t: TermRef<'_>| {
                    let mut s = BTreeSet::new();
                    term_vars(v, rule, t, &mut s);
                    s
                };
                let known = |s: &BTreeSet<String>, bound: &BTreeSet<String>| s.iter().all(|x| bound.contains(x) || filtered.contains(x));
                let (va, vb) = (vars(a), vars(b));
                if known(&va, &bound) && known(&vb, &bound) {
                    cmps.push(*l);
                    cmp_texts.push(text);
                    grew = true;
                } else if op == CmpOp::Eq {
                    let assigned = match (a, b) {
                        (TermRef::Var(x), _) if !bound.contains(&name(v, rule, x)) && known(&vb, &bound) => Some(name(v, rule, x)),
                        (_, TermRef::Var(x)) if !bound.contains(&name(v, rule, x)) && known(&va, &bound) => Some(name(v, rule, x)),
                        _ => None,
                    };
                    if let Some(x) = assigned {
                        bound.insert(x);
                        cmps.push(*l);
                        cmp_texts.push(text);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        kept.extend(cmps);
        Some(one_group(v, rule, head, &alias, kept, filters))
    }

    /// The keys that are `top`: from the start, `forced`, or a rule whose head only a `top` filter binds. To a
    /// fixpoint.
    fn tops(&self, forced: &BTreeSet<Key>) -> BTreeSet<Key> {
        let mut top: BTreeSet<Key> = self.shapes.iter().filter(|(_, s)| s.is_none()).map(|(k, _)| k.clone()).collect();
        top.extend(forced.iter().cloned());
        loop {
            let mut grew = false;
            for (key, shapes) in &self.shapes {
                let Some(shapes) = shapes else { continue };
                if top.contains(key) {
                    continue;
                }
                let unbound = shapes.iter().any(|s| !s.bound.contains(&s.head) && !s.filters.iter().any(|(k, x)| x == &s.head && !top.contains(k) && self.shapes.contains_key(k)));
                if unbound {
                    top.insert(key.clone());
                    grew = true;
                }
            }
            if !grew {
                return top;
            }
        }
    }
}

/// Of a superset rule's literals, the shortest chain from the head to one that cuts: a filter (another superset) or
/// an atom with a constant. Any part of a rule's body that binds the head is a superset of its column, so the chain
/// alone is one; the rest only narrows it, at the cost of joining it. Joined whole, independent groups meeting at a
/// node enumerate every combination of their rows (`within(c1, n), within(c2, n), within(c3, n)`: 475 M rows on one
/// file). With nothing that cuts, the head is left unbound: the superset is everything (`top`).
fn one_group<'v>(v: &Version, rule: RuleRef<'v>, head: String, alias: &BTreeMap<String, String>, kept: Vec<LitRef<'v>>, filters: Vec<(Key, String)>) -> Shape<'v> {
    let h = find(alias, &head);
    let n_kept = kept.len();
    let mut items: Vec<Option<BTreeSet<String>>> = kept
        .iter()
        .map(|l| match l.view() {
            L::Atom(a) => Some(atom_vars(v, rule, a).iter().map(|x| find(alias, x)).collect()),
            L::Nearest { n, a, .. } => Some([name(v, rule, n), name(v, rule, a)].iter().map(|x| find(alias, x)).collect()),
            _ => None,
        })
        .collect();
    items.extend(filters.iter().map(|(_, x)| Some(BTreeSet::from([find(alias, x)]))));
    let cuts = |i: usize| -> usize {
        match kept.get(i).map(|l| l.view()) {
            Some(L::Atom(a)) => a.args().filter(|t| is_const(*t)).count(),
            Some(_) => 0,
            None => 2,
        }
    };
    let mut via_var: BTreeMap<String, Option<usize>> = BTreeMap::from([(h.clone(), None)]);
    let mut via_item: BTreeMap<usize, String> = BTreeMap::new();
    let mut layer: Vec<String> = vec![h.clone()];
    let mut seed: Option<usize> = None;
    while seed.is_none() && !layer.is_empty() {
        let mut next = Vec::new();
        let mut found: Vec<usize> = Vec::new();
        for x in &layer {
            for (i, it) in items.iter().enumerate() {
                let Some(vars) = it else { continue };
                if via_item.contains_key(&i) || !vars.contains(x) {
                    continue;
                }
                via_item.insert(i, x.clone());
                if cuts(i) > 0 {
                    found.push(i);
                }
                for w in vars {
                    if !via_var.contains_key(w) {
                        via_var.insert(w.clone(), Some(i));
                        next.push(w.clone());
                    }
                }
            }
        }
        seed = found.into_iter().max_by_key(|i| (cuts(*i), std::cmp::Reverse(*i)));
        layer = next;
    }
    let mut on_path: BTreeSet<usize> = BTreeSet::new();
    let mut at = seed;
    while let Some(i) = at {
        on_path.insert(i);
        at = via_item.get(&i).and_then(|x| via_var.get(x).copied().flatten());
    }
    let keep = |i: usize| on_path.contains(&i) || (i < n_kept && items[i].is_none());
    let chosen: Vec<LitRef<'v>> = kept.iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, l)| *l).collect();
    let filters: Vec<(Key, String)> = filters.into_iter().enumerate().filter(|(i, _)| on_path.contains(&(n_kept + i))).map(|(_, f)| f).collect();
    let filtered = |x: &String| filters.iter().any(|(_, f)| f == x);
    let mut bound: BTreeSet<String> = BTreeSet::new();
    for l in &chosen {
        if let L::Atom(a) = l.view() {
            bound.extend(atom_vars(v, rule, a));
        }
    }
    loop {
        let before = bound.len();
        for l in &chosen {
            if let L::Cmp { l: a, op: CmpOp::Eq, r: b } = l.view() {
                let (x, r) = match (a, b) {
                    (TermRef::Var(x), r) => (x, r),
                    (r, TermRef::Var(x)) => (x, r),
                    _ => continue,
                };
                let mut vs = BTreeSet::new();
                term_vars(v, rule, r, &mut vs);
                if vs.iter().all(|y| bound.contains(y) || filtered(y)) {
                    bound.insert(name(v, rule, x));
                }
            }
        }
        for l in &chosen {
            if let L::Nearest { n, a, .. } = l.view() && bound.contains(&name(v, rule, n)) {
                bound.insert(name(v, rule, a));
            }
        }
        if bound.len() == before {
            break;
        }
    }
    let chosen: Vec<LitRef<'v>> = chosen
        .into_iter()
        .filter(|l| match l.view() {
            L::Cmp { l: a, r: b, .. } => {
                let mut vs = BTreeSet::new();
                term_vars(v, rule, a, &mut vs);
                term_vars(v, rule, b, &mut vs);
                vs.iter().all(|y| bound.contains(y) || filtered(y))
            }
            _ => true,
        })
        .collect();
    Shape { rule, head, kept: chosen, filters, bound }
}

fn may_name((rel, at): &Key) -> String {
    format!("dm_{rel}_{at}")
}

/// The name of the relation of a file-local atom's distinct values at column `at`, the other columns its constants or
/// anything: a filter that tests a row exists. Joined as the atom itself, a filter on a column of many rows per key
/// (`fz_b(_, p)`, siblings per parent) enumerates them all for every row it filters. The constants are hashed as the
/// tree's terms printed (`{j}=Int(1)`, `{j}=Str("s")`), so the names stay those stored programs hold.
fn key_name(v: &Version, rel: &str, at: usize, consts: &[(usize, Const)]) -> String {
    use std::hash::{Hash, Hasher};
    let constants: Vec<String> = consts
        .iter()
        .filter(|(j, _)| *j != at)
        .map(|(j, c)| match c {
            Const::Int(n) => format!("{j}=Int({n})"),
            Const::Str(a) => format!("{j}=Str({:?})", v.text(*a)),
            // As the tree's `{t:?}` of a `Term::Branded`.
            Const::Branded(b) => format!("{j}=Branded({b:?})"),
        })
        .collect();
    if constants.is_empty() {
        format!("xk_{rel}_{at}")
    } else {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        constants.hash(&mut h);
        format!("xk_{rel}_{at}_{:08x}", h.finish() as u32)
    }
}

/// A rule the cut makes.
enum Made<'v> {
    /// `dm_<key>(head) :- kept…, dm_<filter>(v)…`, of `rule`.
    Superset { name: String, rule: RuleRef<'v>, head: String, kept: Vec<LitRef<'v>>, filters: Vec<(String, String)> },
    /// `xk_…(v) :- rel(…, v, …)`, the other columns its constants or anything.
    Key { name: String, rel: String, width: usize, at: usize, consts: Vec<(usize, Const)> },
    /// `xd_<rel>(vars) :- rel(vars), constants…, filters…`, of reader `rule`.
    Cut { name: String, rule: RuleRef<'v>, rel: String, vars: Vec<String>, consts: Vec<(String, TermRef<'v>)>, filters: Vec<(String, String)> },
}

/// A round's program: the readers' atoms to rename (by node), the rules made, the declarations and the keys.
struct Plan<'v> {
    renamed: HashMap<u32, String>,
    made: Vec<Made<'v>>,
    decls: BTreeMap<String, Vec<(String, String)>>,
    keys: Vec<String>,
}

struct ReadCut<'p>(&'p HashMap<u32, String>);

impl Rewrite for ReadCut<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.0.get(&a.node()).map(|n| (n.clone(), a.neg(), a.gated()))
    }
}

/// The round's program, into a derivation of `input` (`onto` false: every rule, in order), or onto an edit of it
/// (`onto`: the readers removed and written again, the rest left as they are).
fn write(b: &mut Builder<'_>, input: &Version, plan: &Plan<'_>, onto: bool) -> Result<(), String> {
    for (rel, cols) in &plan.decls {
        let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.fields().declare(rel, &cols);
    }
    for r in input.rules() {
        if r.body().any(|l| plan.renamed.contains_key(&l.node())) {
            if onto {
                b.remove(r.id());
            }
            b.rewrite(input, r, &mut ReadCut(&plan.renamed)).finish()?;
        } else if !onto {
            b.keep(input, r);
        }
    }
    for m in &plan.made {
        match m {
            Made::Superset { name, rule, head, kept, filters } => {
                let mut rb = b.rule().source(rule.id()).head(name.as_str(), &[T::Var(head)]);
                for l in kept {
                    rb = rb.lit(input, *rule, *l);
                }
                for (rel, x) in filters {
                    rb = rb.atom(rel.as_str(), &[T::Var(x)]);
                }
                rb.finish()?;
            }
            Made::Key { name, rel, width, at, consts } => {
                let args: Vec<T<'_>> = (0..*width)
                    .map(|j| match consts.iter().find(|(k, _)| *k == j) {
                        _ if j == *at => T::Var("v"),
                        Some((_, Const::Int(n))) => T::Int(*n),
                        Some((_, Const::Str(a))) => T::StrAtom(*a),
                        Some((_, Const::Branded(b))) => T::Branded(*b),
                        None => T::Wild,
                    })
                    .collect();
                b.rule().head(name.as_str(), &[T::Var("v")]).atom(rel.as_str(), &args).finish()?;
            }
            Made::Cut { name, rule, rel, vars, consts, filters } => {
                let args: Vec<T<'_>> = vars.iter().map(|x| T::Var(x)).collect();
                let mut rb = b.rule().source(rule.id()).head(name.as_str(), &args).atom(rel.as_str(), &args);
                for (x, c) in consts {
                    rb = rb.cmp(T::Var(x), CmpOp::Eq, super::super::passes::t_of(input, *rule, *c));
                }
                for (f, x) in filters {
                    rb = rb.atom(f.as_str(), &[T::Var(x)]);
                }
                rb.finish()?;
            }
        }
    }
    if !plan.keys.is_empty() {
        let words: Vec<(&str, &str)> = plan.keys.iter().map(|k| (k.as_str(), "no_inline")).collect();
        b.fields().add_qualifiers(&words);
    }
    Ok(())
}

impl ProgramPass for DemandExports {
    const NAME: &'static str = "cut the exports to their readers' demand";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        if !on() {
            self.lines = vec!["off (RERAIL_EXPORT_DEMAND=off)".into()];
            return Ok(());
        }
        let v: &Arc<Version> = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let split = cx.get::<FileLocal>()?;
        let (local, locations, same_doc) = (&split.local, &split.locations, &split.same_doc);
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
        let mut rules_of: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::new();
        for r in &rules {
            rules_of.entry(head(r)).or_default().push(*r);
        }
        let mut pass = Pass { v, f, local, locations, same_doc, rules_of, shapes: BTreeMap::new() };
        let cross: Vec<RuleRef<'_>> = rules.iter().copied().filter(|r| !local.contains(head(r))).collect();
        let mut whole: BTreeSet<String> = f.natives().iter().filter(|n| !local.contains(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r).to_owned()).collect();
        whole.extend(f.outputs().map(str::to_owned));
        whole.extend(f.lattices().iter().map(|l| v.rel_name(l.rel).to_owned()));
        whole.extend(f.gates().iter().map(|g| v.text(g.key).to_owned()));
        for r in &cross {
            for l in r.body() {
                match l.view() {
                    L::Atom(a) if a.neg() => {
                        whole.insert(v.rel_name(a.rel()).to_owned());
                    }
                    L::Agg { atom, .. } => {
                        whole.insert(v.rel_name(atom.rel()).to_owned());
                    }
                    _ => {}
                }
            }
        }
        let projection = |rel: &str| {
            let mut of = rules.iter().filter(|r| head(r) == rel).peekable();
            of.peek().is_some()
                && of.all(|r| {
                    let body: Vec<LitRef<'_>> = r.body().collect();
                    matches!(body.as_slice(), [l] if matches!(l.view(), L::Atom(a) if !a.neg() && f.is_input(v.rel_name(a.rel()))))
                })
        };
        // Each export's reads: (reader, literal) and each read's filters.
        let mut reads: BTreeMap<String, Vec<(RuleRef<'_>, LitRef<'_>, Vec<Filter>)>> = BTreeMap::new();
        let mut uncut: BTreeSet<String> = BTreeSet::new();
        for &rule in &cross {
            let body: Vec<LitRef<'_>> = rule.body().collect();
            for (li, l) in body.iter().enumerate() {
                let L::Atom(e) = l.view() else { continue };
                let rel = v.rel_name(e.rel());
                if e.neg() || !local.contains(rel) || whole.contains(rel) || projection(rel) {
                    continue;
                }
                if e.args().any(|t| matches!(t, TermRef::Call(..) | TermRef::Top)) {
                    uncut.insert(rel.to_owned());
                    continue;
                }
                let mut filters = Vec::new();
                for (p, t) in e.args().enumerate() {
                    let TermRef::Var(s) = t else { continue };
                    if !pass.same(rel, p) {
                        continue;
                    }
                    for (oi, o) in body.iter().enumerate() {
                        let L::Atom(a) = o.view() else { continue };
                        if oi == li || a.neg() {
                            continue;
                        }
                        let arel = v.rel_name(a.rel());
                        for (j, u) in a.args().enumerate() {
                            if u.var() != Some(s) {
                                continue;
                            }
                            if pass.usable(arel) && pass.same(arel, j) {
                                let consts = a
                                    .args()
                                    .enumerate()
                                    .filter_map(|(k, t)| match t {
                                        TermRef::Int(n) if k != j => Some((k, Const::Int(n))),
                                        TermRef::Str(x) if k != j => Some((k, Const::Str(x))),
                                        TermRef::Branded(b) if k != j => Some((k, Const::Branded(b))),
                                        _ => None,
                                    })
                                    .collect();
                                filters.push(Filter::Atom { rel: arel.to_owned(), width: a.args().count(), at: j, consts, v: name(v, rule, s) });
                            } else if !pass.usable(arel) && pass.located(arel, j) {
                                filters.push(Filter::Key((arel.to_owned(), j), name(v, rule, s)));
                            }
                        }
                    }
                }
                reads.entry(rel.to_owned()).or_default().push((rule, *l, filters));
            }
        }
        let keys: Vec<Key> = reads.values().flatten().flat_map(|(_, _, fs)| fs.iter().filter_map(|f| if let Filter::Key(k, _) = f { Some(k.clone()) } else { None })).collect();
        for k in keys {
            pass.explore(k);
        }
        let mut forced: BTreeSet<Key> = BTreeSet::new();
        let mut excluded: BTreeSet<String> = BTreeSet::new();
        let mut first_stray: Option<String> = None;
        for round in 1..=ROUNDS {
            let top = pass.tops(&forced);
            let useful = |f: &Filter| match f {
                Filter::Atom { .. } => true,
                Filter::Key(k, _) => !top.contains(k),
            };
            let cut: Vec<String> = reads
                .iter()
                .filter(|(rel, rs)| {
                    !uncut.contains(*rel)
                        && !excluded.contains(*rel)
                        && rs.iter().all(|(_, l, fs)| {
                            let constant = matches!(l.view(), L::Atom(e) if e.args().any(is_const));
                            constant || fs.iter().any(useful)
                        })
                })
                .map(|(rel, _)| rel.clone())
                .collect();
            if cut.is_empty() {
                self.lines = vec![format!("no export has a demand to cut it by{}", first_stray.map_or_else(String::new, |s| format!(" (what strayed first: {s})")))];
                return Ok(());
            }
            let mut needed: BTreeSet<Key> = BTreeSet::new();
            let mut work: Vec<Key> = cut
                .iter()
                .flat_map(|rel| {
                    reads[rel].iter().flat_map(|(_, _, fs)| {
                        fs.iter().filter_map(|f| match f {
                            Filter::Key(k, _) if !top.contains(k) => Some(k.clone()),
                            _ => None,
                        })
                    })
                })
                .collect();
            while let Some(k) = work.pop() {
                if needed.insert(k.clone()) && let Some(Some(shapes)) = pass.shapes.get(&k) {
                    work.extend(shapes.iter().flat_map(|s| s.filters.iter().map(|(k, _)| k.clone())).filter(|k| !top.contains(k)));
                }
            }
            let mut plan = Plan { renamed: HashMap::new(), made: Vec::new(), decls: BTreeMap::new(), keys: Vec::new() };
            for key in &needed {
                let Some(Some(shapes)) = pass.shapes.get(key) else { continue };
                let ty = f.column_type(&key.0, key.1).unwrap_or("number").to_owned();
                plan.decls.insert(may_name(key), vec![("v".into(), ty)]);
                for s in shapes {
                    let filters = s.filters.iter().filter(|(k, _)| !top.contains(k)).map(|(k, x)| (may_name(k), x.clone())).collect();
                    plan.made.push(Made::Superset { name: may_name(key), rule: s.rule, head: s.head.clone(), kept: s.kept.clone(), filters });
                }
            }
            let mut lines = Vec::new();
            let mut keys_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for rel in &cut {
                let xd = format!("xd_{rel}");
                let cols = f.columns(rel).unwrap_or_default().into_iter().map(|(n, t)| (n.to_owned(), t.to_owned())).collect();
                plan.decls.insert(xd.clone(), cols);
                let mut seen: BTreeSet<String> = BTreeSet::new();
                for (rule, l, fs) in &reads[rel] {
                    let L::Atom(e) = l.view() else { continue };
                    let mut vars = Vec::new();
                    let mut consts = Vec::new();
                    for (p, t) in e.args().enumerate() {
                        match t {
                            TermRef::Var(s) => vars.push(name(v, *rule, s)),
                            TermRef::Wild => vars.push(format!("xd_w{p}")),
                            c => {
                                vars.push(format!("xd_c{p}"));
                                consts.push((format!("xd_c{p}"), c));
                            }
                        }
                    }
                    let mut filters = Vec::new();
                    for fl in fs.iter().filter(|f| useful(f)) {
                        match fl {
                            Filter::Atom { rel: arel, width, at, consts: ac, v: x } => {
                                let key = key_name(v, arel, *at, ac);
                                if !plan.decls.contains_key(&key) {
                                    let ty = f.column_type(arel, *at).unwrap_or("number").to_owned();
                                    plan.decls.insert(key.clone(), vec![("v".into(), ty)]);
                                    plan.made.push(Made::Key { name: key.clone(), rel: arel.clone(), width: *width, at: *at, consts: ac.clone() });
                                }
                                keys_of.entry(key.clone()).or_default().insert(rel.clone());
                                filters.push((key, x.clone()));
                            }
                            Filter::Key(k, x) => filters.push((may_name(k), x.clone())),
                        }
                    }
                    // The rule's text, as the tree's dedupe reads it.
                    let text = format!(
                        "{xd}({}) :- {rel}({}){}{}",
                        vars.join(","),
                        vars.join(","),
                        consts.iter().map(|(x, c)| format!(", {x} = {}", term_text(v, *rule, *c))).collect::<String>(),
                        filters.iter().map(|(f, x)| format!(", {f}({x})")).collect::<String>()
                    );
                    if seen.insert(text) {
                        plan.made.push(Made::Cut { name: xd.clone(), rule: *rule, rel: rel.clone(), vars, consts, filters });
                    }
                }
                lines.push(format!("{rel}: {} reads, cut by {} rules", reads[rel].len(), seen.len()));
                for (_, l, _) in &reads[rel] {
                    plan.renamed.insert(l.node(), xd.clone());
                }
            }
            plan.keys = keys_of.keys().cloned().collect();
            // The cut and its supersets must split as file-local: this round's program, split.
            // An overlay of the input holding the round's change, its split carried across it (`passes::candidate`).
            let mut cb = Builder::on(v);
            write(&mut cb, v, &plan, true)?;
            let candidate = cb.finish_overlay()?;
            let after = manager::carried::<FileLocal>(v, &candidate)?;
            let stray: Vec<String> = cut.iter().map(|r| format!("xd_{r}")).chain(needed.iter().map(may_name)).chain(keys_of.keys().cloned()).filter(|r| !after.local.contains(r)).collect();
            if !stray.is_empty() {
                if first_stray.is_none() {
                    let why = after.causes.get(&stray[0]).cloned().unwrap_or_default();
                    first_stray = Some(format!("{}: {why}", stray[0]));
                }
                for r in &stray {
                    if let Some(e) = r.strip_prefix("xd_") {
                        excluded.insert(e.to_owned());
                    } else if let Some(users) = keys_of.get(r) {
                        excluded.extend(users.iter().cloned());
                    } else {
                        forced.extend(needed.iter().filter(|k| may_name(k) == *r).cloned());
                    }
                }
                continue;
            }
            lines.insert(
                0,
                format!(
                    "{} exports cut by their readers' demand, through {} supersets ({} top of {} explored; round {round}, {} cuts and {} supersets left out as not file-local{})",
                    cut.len(),
                    needed.len(),
                    top.len(),
                    pass.shapes.len(),
                    excluded.len(),
                    forced.len(),
                    first_stray.map_or_else(String::new, |s| format!(", first {s}"))
                ),
            );
            self.lines = lines;
            write(w.derive()?, v, &plan, false)?;
            return Ok(());
        }
        self.lines = vec![format!("undone: still not file-local after {ROUNDS} rounds ({} cuts, {} supersets left out)", excluded.len(), forced.len())];
        Ok(())
    }
}
