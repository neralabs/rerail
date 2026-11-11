//! Export demand: an export holds only the rows its cross-file readers can use.
//!
//! A file-local relation that a cross-file rule reads is an export. The chunks make it in full and hand all of it
//! on, and the per-file memo keeps it. Its reader often joins it with a relation that names only a few of its rows:
//!
//! ```text
//! fl_later(n, c1) :- within(c1, n), rr_nearest(c1, s), s = n.          -- every (scope, descendant) pair
//! later(n, N)     :- demand_later(n, N), fl_later(n, c1), binds(c1, N). -- the scopes demand_later names
//! ```
//!
//! `fl_later` is 278 k rows on 20 files; the reader uses the scopes `demand_later` names. `demand_later` is
//! cross-file (its `N` is another file's), but its `n` is derived from this file's rows: each of its rules, cut to
//! the literals in `n`'s document, is a file-local rule for a superset of its `n` column:
//!
//! ```text
//! demand_later(s1, N) :- demand_alt(n, N), agg13(n, s1).   -- cross-file
//! dm_demand_later_0(s1) :- dm_demand_alt_0(n), agg13(n, s1). -- file-local: every s1 it can name, and more
//! ```
//!
//! The export is then handed on cut to its readers' demand (`xd_fl_later(n, c1) :- fl_later(n, c1),
//! dm_demand_later_0(n)`), and its readers read that. The cut keeps every row a reader joins: each filter is implied
//! by a literal of the reader.
//!
//! Why the superset fires in the right chunk: a demand's column holds a location (a node or a document). Its rule
//! keeps only the literals joined to that location through same-document columns (`same_doc`), so each lies in the
//! location's document, which is one chunk's. Literals of other documents, negations and aggregates are dropped:
//! dropping a literal only widens the superset. A cross-file literal on a kept location is its own superset,
//! `dm_<rel>_<column>`, made the same way (recursively; a cycle is an ordinary file-local recursion). A rule whose
//! location the kept literals do not bind makes the superset everything (`top`): it filters nothing, and is left out.
//!
//! An export is left whole when a reader reads it negatively, in an aggregate, natively, or with nothing to cut it
//! by; when it is an output, a lattice or a gate's; and when the cut relations do not split as file-local.

use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The most supersets made: past it, a demand is `top` (the readers are cut by what was made).
const MAX_KEYS: usize = 2000;

/// Whether the cut is on: it is, unless `RERAIL_EXPORT_DEMAND=off` (for comparison). The prepared program's key
/// follows the program, so a changed choice is another file-local part and a memo miss.
pub(crate) fn on() -> bool {
    std::env::var("RERAIL_EXPORT_DEMAND").map_or(true, |v| v != "off")
}

type Key = (String, usize);

/// A superset rule before the supersets that are `top` are known: its head variable, the literals kept, and the
/// supersets it filters by, `(key, variable)`.
struct Shape {
    head: String,
    kept: Vec<Literal>,
    filters: Vec<(Key, String)>,
    /// The variables the kept atoms and comparisons bind, without the filters.
    bound: BTreeSet<String>,
}

struct Pass<'a> {
    prog: &'a Program,
    local: &'a BTreeSet<String>,
    locations: &'a BTreeMap<String, Vec<usize>>,
    same_doc: &'a BTreeMap<String, Vec<usize>>,
    rules_of: HashMap<&'a str, Vec<&'a Rule>>,
    shapes: BTreeMap<Key, Option<Vec<Shape>>>,
}

impl<'a> Pass<'a> {
    /// A relation the chunk holds by document: file-local or a base input of the chunk.
    fn usable(&self, rel: &str) -> bool {
        (self.local.contains(rel) || self.prog.inputs.iter().any(|i| i == rel))
            && !matches!(rel, "reference_edge" | "reference_open")
            && self.same_doc.contains_key(rel)
    }

    fn same(&self, rel: &str, at: usize) -> bool {
        self.same_doc.get(rel).is_some_and(|c| c.contains(&at))
    }

    fn located(&self, rel: &str, at: usize) -> bool {
        self.locations.get(rel).is_some_and(|c| c.contains(&at))
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

    fn shapes_of(&self, (rel, at): &Key) -> Option<Vec<Shape>> {
        if self.prog.natives.contains_key(rel) || self.prog.lattice.contains_key(rel) || self.local.contains(rel) {
            return None;
        }
        let rules = self.rules_of.get(rel.as_str())?;
        rules.iter().map(|r| self.shape(r, *at)).collect()
    }

    /// One rule of a cross-file relation, cut to the literals in the document of its column `at`.
    fn shape(&self, rule: &Rule, at: usize) -> Option<Shape> {
        let Some(Term::Var(head)) = rule.head.args.get(at) else { return None };
        // Variables equal by `$a = $b` are one location.
        let mut alias: BTreeMap<String, String> = BTreeMap::new();
        fn find(alias: &BTreeMap<String, String>, v: &str) -> String {
            let mut v = v.to_owned();
            while let Some(n) = alias.get(&v) {
                v = n.clone();
            }
            v
        }
        for l in &rule.body {
            if let Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } = l {
                let (a, b) = (find(&alias, a), find(&alias, b));
                if a != b {
                    alias.insert(a, b);
                }
            }
        }
        let mut reach: BTreeSet<String> = BTreeSet::from([find(&alias, head)]);
        let mut keep = vec![false; rule.body.len()];
        loop {
            let mut grew = false;
            for (i, l) in rule.body.iter().enumerate() {
                if keep[i] {
                    continue;
                }
                let joined: Vec<String> = match l {
                    Literal::Atom(a) if !a.neg && self.usable(&a.rel) => a
                        .args
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| self.same(&a.rel, *j))
                        .filter_map(|(_, t)| match t {
                            Term::Var(v) => Some(find(&alias, v)),
                            _ => None,
                        })
                        .collect(),
                    Literal::Nearest { n, a, .. } => vec![find(&alias, n), find(&alias, a)],
                    _ => continue,
                };
                if joined.iter().any(|v| reach.contains(v)) {
                    keep[i] = true;
                    reach.extend(joined);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut kept: Vec<Literal> = Vec::new();
        let mut bound: BTreeSet<String> = BTreeSet::new();
        for (i, l) in rule.body.iter().enumerate() {
            if keep[i] && let Literal::Atom(a) = l {
                bound.extend(a.vars());
                kept.push(l.clone());
            }
        }
        // A nearest ancestor is found from a bound node.
        for (i, l) in rule.body.iter().enumerate() {
            if keep[i] && let Literal::Nearest { n, a, .. } = l && bound.contains(n) {
                bound.insert(a.clone());
                kept.push(l.clone());
            }
        }
        // The cross-file literals on a location of this document: their own supersets.
        let mut filters: Vec<(Key, String)> = Vec::new();
        for l in &rule.body {
            if let Literal::Atom(a) = l && !a.neg && !self.usable(&a.rel) {
                for (j, t) in a.args.iter().enumerate() {
                    if let Term::Var(v) = t && reach.contains(&find(&alias, v)) && self.located(&a.rel, j) {
                        filters.push(((a.rel.to_string(), j), v.clone()));
                    }
                }
            }
        }
        // Comparisons over bound variables (an equality binds its other side), kept in order.
        let mut cmps: Vec<Literal> = Vec::new();
        let filtered: BTreeSet<String> = filters.iter().map(|(_, v)| v.clone()).collect();
        loop {
            let mut grew = false;
            for l in &rule.body {
                let Literal::Cmp { l: a, op, r: b } = l else { continue };
                if cmps.contains(l) {
                    continue;
                }
                let vars = |t: &Term| {
                    let mut s = BTreeSet::new();
                    t.vars(&mut s);
                    s
                };
                let known = |s: &BTreeSet<String>| s.iter().all(|v| bound.contains(v) || filtered.contains(v));
                let (va, vb) = (vars(a), vars(b));
                if known(&va) && known(&vb) {
                    cmps.push(l.clone());
                    grew = true;
                } else if *op == "=" {
                    let assigned = match (a, b) {
                        (Term::Var(x), _) if !bound.contains(x) && known(&vb) => Some(x.clone()),
                        (_, Term::Var(x)) if !bound.contains(x) && known(&va) => Some(x.clone()),
                        _ => None,
                    };
                    if let Some(x) = assigned {
                        bound.insert(x);
                        cmps.push(l.clone());
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        kept.extend(cmps);
        Some(one_group(head, &alias, kept, filters))
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
                let unbound = shapes.iter().any(|s| {
                    !s.bound.contains(&s.head) && !s.filters.iter().any(|(k, v)| v == &s.head && !top.contains(k) && self.shapes.contains_key(k))
                });
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
fn one_group(head: &str, alias: &BTreeMap<String, String>, kept: Vec<Literal>, filters: Vec<(Key, String)>) -> Shape {
    fn find(alias: &BTreeMap<String, String>, v: &str) -> String {
        let mut v = v.to_owned();
        while let Some(n) = alias.get(&v) {
            v = n.clone();
        }
        v
    }
    let h = find(alias, head);
    // The items a chain runs through: atoms, nearest ancestors and filters, each with its variables.
    let n_kept = kept.len();
    let mut items: Vec<Option<BTreeSet<String>>> = kept
        .iter()
        .map(|l| match l {
            Literal::Atom(_) | Literal::Nearest { .. } => Some(l.vars().iter().map(|v| find(alias, v)).collect()),
            _ => None,
        })
        .collect();
    items.extend(filters.iter().map(|(_, v)| Some(BTreeSet::from([find(alias, v)]))));
    let cuts = |i: usize| -> usize {
        match kept.get(i) {
            Some(Literal::Atom(a)) => a.args.iter().filter(|t| matches!(t, Term::Int(_) | Term::Str(_))).count(),
            Some(_) => 0,
            None => 2,
        }
    };
    // Breadth first from the head, through variables and items, to the nearest item that cuts.
    let mut via_var: BTreeMap<String, Option<usize>> = BTreeMap::from([(h.clone(), None)]);
    let mut via_item: BTreeMap<usize, String> = BTreeMap::new();
    let mut layer: Vec<String> = vec![h.clone()];
    let mut seed: Option<usize> = None;
    while seed.is_none() && !layer.is_empty() {
        let mut next = Vec::new();
        let mut found: Vec<usize> = Vec::new();
        for v in &layer {
            for (i, it) in items.iter().enumerate() {
                let Some(vars) = it else { continue };
                if via_item.contains_key(&i) || !vars.contains(v) {
                    continue;
                }
                via_item.insert(i, v.clone());
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
        at = via_item.get(&i).and_then(|v| via_var.get(v).copied().flatten());
    }
    let keep = |i: usize| on_path.contains(&i) || (i < n_kept && items[i].is_none());
    let chosen: Vec<Literal> = kept.iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, l)| l.clone()).collect();
    let filters: Vec<(Key, String)> = filters.into_iter().enumerate().filter(|(i, _)| on_path.contains(&(n_kept + i))).map(|(_, f)| f).collect();
    let mut bound: BTreeSet<String> = BTreeSet::new();
    for l in &chosen {
        if let Literal::Atom(a) = l {
            bound.extend(a.vars());
        }
    }
    // Assignments among the chosen bind their variable.
    loop {
        let before = bound.len();
        for l in &chosen {
            if let Literal::Cmp { l: Term::Var(x), op: "=", r } | Literal::Cmp { l: r, op: "=", r: Term::Var(x) } = l {
                let mut vs = BTreeSet::new();
                r.vars(&mut vs);
                if vs.iter().all(|v| bound.contains(v) || filters.iter().any(|(_, f)| f == v)) {
                    bound.insert(x.clone());
                }
            }
        }
        for l in &chosen {
            if let Literal::Nearest { n, a, .. } = l && bound.contains(n) {
                bound.insert(a.clone());
            }
        }
        if bound.len() == before {
            break;
        }
    }
    // A comparison whose variables the chosen group does not bind is dropped (a superset needs none).
    let chosen: Vec<Literal> = chosen
        .into_iter()
        .filter(|l| match l {
            Literal::Cmp { l: a, r: b, .. } => {
                let mut vs = BTreeSet::new();
                a.vars(&mut vs);
                b.vars(&mut vs);
                vs.iter().all(|v| bound.contains(v) || filters.iter().any(|(_, f)| f == v))
            }
            _ => true,
        })
        .collect();
    Shape { head: head.to_owned(), kept: chosen, filters, bound }
}

fn may_name((rel, at): &Key) -> String {
    format!("dm_{rel}_{at}")
}

/// A reader's filters on an export it reads at `i`: `(variable, filter)`, a filter being a file-local atom or a key.
enum Filter {
    /// A file-local atom on the variable at its column `.1`: read through its distinct keys there (`key_rel`).
    Atom(Atom, usize, String),
    Key(Key, String),
}

/// The relation of `a`'s distinct values at column `at`, the other columns its constants or anything: a filter that
/// tests a row exists. Joined as the atom itself, a filter on a column of many rows per key (`fz_b(_, p)`, siblings
/// per parent) enumerates them all for every row it filters.
fn key_rel(a: &Atom, at: usize) -> (String, Rule) {
    use std::hash::{Hash, Hasher};
    let constants: Vec<String> = a.args.iter().enumerate().filter(|(j, t)| *j != at && matches!(t, Term::Int(_) | Term::Str(_))).map(|(j, t)| format!("{j}={t:?}")).collect();
    let name = if constants.is_empty() {
        format!("xk_{}_{at}", a.rel)
    } else {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        constants.hash(&mut h);
        format!("xk_{}_{at}_{:08x}", a.rel, h.finish() as u32)
    };
    let args = a.args.iter().enumerate().map(|(j, t)| if j == at { Term::Var("v".into()) } else if matches!(t, Term::Int(_) | Term::Str(_)) { t.clone() } else { Term::Wild }).collect();
    let rule = Rule { head: Atom { rel: name.as_str().into(), args: vec![Term::Var("v".into())], neg: false, gated: false }, body: vec![Literal::Atom(Atom { rel: a.rel.clone(), args, neg: false, gated: false })] };
    (name, rule)
}

/// Cuts each export to its cross-file readers' demand. Returns a line per export cut, and why the pass made none.
pub(crate) fn demand_exports(prog: &mut Program) -> Vec<String> {
    if !on() {
        return vec!["off (RERAIL_EXPORT_DEMAND=off)".into()];
    }
    let (local, locations, same_doc) = super::split_located(prog);
    let original = prog.clone();
    let mut rules_of: HashMap<&str, Vec<&Rule>> = HashMap::new();
    for r in &original.rules {
        rules_of.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let mut pass = Pass { prog: &original, local: &local, locations: &locations, same_doc: &same_doc, rules_of, shapes: BTreeMap::new() };
    let cross: Vec<usize> = (0..original.rules.len()).filter(|i| !local.contains(original.rules[*i].head.rel.as_str())).collect();
    // What may not be cut: read by a native, negated, in an aggregate, an output, a lattice, a gate's.
    let mut whole: BTreeSet<String> = original.natives.values().flat_map(|n| n.reads()).map(str::to_owned).collect();
    whole.extend(original.outputs.iter().cloned());
    whole.extend(original.lattice.keys().cloned());
    whole.extend(original.gates.iter().map(|(r, _)| r.clone()));
    for &i in &cross {
        for l in &original.rules[i].body {
            match l {
                Literal::Atom(a) if a.neg => {
                    whole.insert(a.rel.to_string());
                }
                Literal::Agg { atom, .. } => {
                    whole.insert(atom.rel.to_string());
                }
                _ => {}
            }
        }
    }
    // A projection of one input the cross part derives itself (`programs`): not an export.
    let projection = |rel: &str| {
        let mut rules = original.rules.iter().filter(|r| r.head.rel == rel).peekable();
        rules.peek().is_some() && rules.all(|r| matches!(r.body.as_slice(), [Literal::Atom(a)] if !a.neg && original.inputs.iter().any(|x| x == &a.rel)))
    };
    // Each export's reads: (rule, literal) of its cross-file readers, and each read's filters.
    let mut reads: BTreeMap<String, Vec<(usize, usize, Vec<Filter>)>> = BTreeMap::new();
    let mut uncut: BTreeSet<String> = BTreeSet::new();
    for &ri in &cross {
        let rule = &original.rules[ri];
        for (li, l) in rule.body.iter().enumerate() {
            let Literal::Atom(e) = l else { continue };
            let rel = e.rel.as_str();
            if e.neg || !local.contains(rel) || whole.contains(rel) || projection(rel) {
                continue;
            }
            if e.args.iter().any(|t| matches!(t, Term::Call(..) | Term::Top)) {
                uncut.insert(rel.to_owned());
                continue;
            }
            let mut filters = Vec::new();
            for (p, t) in e.args.iter().enumerate() {
                let Term::Var(v) = t else { continue };
                if !pass.same(rel, p) {
                    continue;
                }
                for (oi, o) in rule.body.iter().enumerate() {
                    let Literal::Atom(a) = o else { continue };
                    if oi == li || a.neg {
                        continue;
                    }
                    for (j, u) in a.args.iter().enumerate() {
                        if u != &Term::Var(v.clone()) {
                            continue;
                        }
                        if pass.usable(&a.rel) && pass.same(&a.rel, j) {
                            let args = a.args.iter().enumerate().map(|(k, t)| if k == j { t.clone() } else if matches!(t, Term::Int(_) | Term::Str(_)) { t.clone() } else { Term::Wild }).collect();
                            filters.push(Filter::Atom(Atom { rel: a.rel.clone(), args, neg: false, gated: false }, j, v.clone()));
                        } else if !pass.usable(&a.rel) && pass.located(&a.rel, j) {
                            filters.push(Filter::Key((a.rel.to_string(), j), v.clone()));
                        }
                    }
                }
            }
            reads.entry(rel.to_owned()).or_default().push((ri, li, filters));
        }
    }
    for (_, rs) in &reads {
        for (_, _, fs) in rs {
            for f in fs {
                if let Filter::Key(k, _) = f {
                    pass.explore(k.clone());
                }
            }
        }
    }
    // A superset or a cut that does not split as file-local is left out (a superset is `top` then), and the cut is
    // made again without it.
    let mut forced: BTreeSet<Key> = BTreeSet::new();
    let mut excluded: BTreeSet<String> = BTreeSet::new();
    let mut first_stray: Option<String> = None;
    for round in 1..=ROUNDS {
    *prog = original.clone();
    let top = pass.tops(&forced);
    let useful = |f: &Filter| match f {
        Filter::Atom(..) => true,
        Filter::Key(k, _) => !top.contains(k),
    };
    // An export is cut when every read of it has a filter, or a constant.
    let cut: Vec<String> = reads
        .iter()
        .filter(|(rel, rs)| {
            !uncut.contains(*rel)
                && !excluded.contains(*rel)
                && rs.iter().all(|(ri, li, fs)| {
                    let constant = matches!(&original.rules[*ri].body[*li], Literal::Atom(e) if e.args.iter().any(|t| matches!(t, Term::Int(_) | Term::Str(_))));
                    constant || fs.iter().any(&useful)
                })
        })
        .map(|(rel, _)| rel.clone())
        .collect();
    if cut.is_empty() {
        *prog = original;
        return vec![format!("no export has a demand to cut it by{}", first_stray.map_or_else(String::new, |s| format!(" (what strayed first: {s})")))];
    }
    // The supersets the cut reads, with every one they read.
    let mut needed: BTreeSet<Key> = BTreeSet::new();
    let mut work: Vec<Key> = cut
        .iter()
        .flat_map(|rel| reads[rel].iter().flat_map(|(_, _, fs)| fs.iter().filter_map(|f| match f { Filter::Key(k, _) if !top.contains(k) => Some(k.clone()), _ => None })))
        .collect();
    while let Some(k) = work.pop() {
        if needed.insert(k.clone()) && let Some(Some(shapes)) = pass.shapes.get(&k) {
            work.extend(shapes.iter().flat_map(|s| s.filters.iter().map(|(k, _)| k.clone())).filter(|k| !top.contains(k)));
        }
    }
    let mut made: Vec<Rule> = Vec::new();
    let mut decls: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for key in &needed {
        let Some(Some(shapes)) = pass.shapes.get(key) else { continue };
        let ty = original.decls.get(&key.0).and_then(|c| c.get(key.1)).map_or_else(|| "number".to_owned(), |c| c.1.clone());
        decls.insert(may_name(key), vec![("v".into(), ty)]);
        for s in shapes {
            let mut body = s.kept.clone();
            body.extend(s.filters.iter().filter(|(k, _)| !top.contains(k)).map(|(k, v)| Literal::Atom(Atom { rel: may_name(k).as_str().into(), args: vec![Term::Var(v.clone())], neg: false, gated: false })));
            made.push(Rule { head: Atom { rel: may_name(key).as_str().into(), args: vec![Term::Var(s.head.clone())], neg: false, gated: false }, body });
        }
    }
    let mut lines = Vec::new();
    // Each key relation made, by the exports whose cut reads it.
    let mut keys_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rel in &cut {
        let name = format!("xd_{rel}");
        decls.insert(name.clone(), original.decls.get(rel).cloned().unwrap_or_default());
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (ri, li, fs) in &reads[rel] {
            let Literal::Atom(e) = &original.rules[*ri].body[*li] else { continue };
            // The head names every column: a wildcard or a constant of the read becomes a variable.
            let mut head = Vec::new();
            let mut body = Vec::new();
            let mut args = Vec::new();
            for (p, t) in e.args.iter().enumerate() {
                match t {
                    Term::Var(v) => {
                        head.push(t.clone());
                        args.push(Term::Var(v.clone()));
                    }
                    Term::Wild => {
                        let v = format!("xd_w{p}");
                        head.push(Term::Var(v.clone()));
                        args.push(Term::Var(v));
                    }
                    c => {
                        let v = format!("xd_c{p}");
                        head.push(Term::Var(v.clone()));
                        args.push(Term::Var(v.clone()));
                        body.push(Literal::Cmp { l: Term::Var(v), op: "=", r: c.clone() });
                    }
                }
            }
            body.insert(0, Literal::Atom(Atom { rel: e.rel.clone(), args, neg: false, gated: false }));
            for f in fs.iter().filter(|f| useful(f)) {
                body.push(Literal::Atom(match f {
                    Filter::Atom(a, at, v) => {
                        let (key, rule) = key_rel(a, *at);
                        if !decls.contains_key(&key) {
                            let ty = original.decls.get(a.rel.as_str()).and_then(|c| c.get(*at)).map_or_else(|| "number".to_owned(), |c| c.1.clone());
                            decls.insert(key.clone(), vec![("v".into(), ty)]);
                            made.push(rule);
                        }
                        keys_of.entry(key.clone()).or_default().insert(rel.clone());
                        Atom { rel: key.as_str().into(), args: vec![Term::Var(v.clone())], neg: false, gated: false }
                    }
                    Filter::Key(k, v) => Atom { rel: may_name(k).as_str().into(), args: vec![Term::Var(v.clone())], neg: false, gated: false },
                }));
            }
            let rule = Rule { head: Atom { rel: name.as_str().into(), args: head, neg: false, gated: false }, body };
            if seen.insert(rule.to_string()) {
                made.push(rule);
            }
        }
        lines.push(format!("{rel}: {} reads, cut by {} rules", reads[rel].len(), seen.len()));
    }
    // The readers read the cut.
    for rel in &cut {
        for (ri, li, _) in &reads[rel] {
            if let Literal::Atom(e) = &mut prog.rules[*ri].body[*li] {
                e.rel = format!("xd_{rel}").as_str().into();
            }
        }
    }
    prog.rules.extend(made);
    prog.decls.extend(decls);
    // The cut and its supersets must split as file-local, or the cut would hand on more than before.
    let (local_after, _, _) = super::split_located(prog);
    let stray: Vec<String> = cut.iter().map(|r| format!("xd_{r}")).chain(needed.iter().map(may_name)).chain(keys_of.keys().cloned()).filter(|r| !local_after.contains(r)).collect();
    if !stray.is_empty() {
        if first_stray.is_none() {
            let why = super::cross_causes(prog).get(&stray[0]).cloned().unwrap_or_default();
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
    lines.insert(0, format!(
        "{} exports cut by their readers' demand, through {} supersets ({} top of {} explored; round {round}, {} cuts and {} supersets left out as not file-local{})",
        cut.len(),
        needed.len(),
        top.len(),
        pass.shapes.len(),
        excluded.len(),
        forced.len(),
        first_stray.map_or_else(String::new, |s| format!(", first {s}"))
    ));
    return lines;
    }
    *prog = original;
    vec![format!("undone: still not file-local after {ROUNDS} rounds ({} cuts, {} supersets left out)", excluded.len(), forced.len())]
}

/// The rounds a cut is made again in, each without what did not split as file-local.
const ROUNDS: usize = 8;
