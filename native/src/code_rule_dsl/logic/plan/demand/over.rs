//! The negation split's readings over the program's version (t36 item 6): which relations it demands
//! (`candidates`), and each reader's split (`analyse`), as `split_negations` and `split_with` read the tree. The
//! rules are read by their handles, each variable by its name; the program's other fields by `Fields`.

use super::super::super::program_image::{AtomRef, CmpOp, Lit, LitRef, RuleRef, Slot, TermRef, Version};
use super::super::body::{Placed, lit_vars, term_vars, VarSet};
use super::super::fields::Fields;
use super::Split;
use std::collections::{BTreeMap, BTreeSet};

/// A rule of the version, its variables by name.
struct R<'v> {
    v: &'v Version,
    r: RuleRef<'v>,
    lits: Vec<LitRef<'v>>,
}

impl<'v> R<'v> {
    fn of(p: &'v Placed, k: usize) -> R<'v> {
        let r = p.rule(k);
        R { v: &p.v, r, lits: r.body().collect() }
    }
    fn name(&self, s: Slot) -> &'v str {
        self.v.text(self.r.var(s))
    }
    fn names(&self, vs: &VarSet) -> BTreeSet<String> {
        vs.iter().map(|s| self.name(s).to_owned()).collect()
    }
    fn lit_vars(&self, i: usize) -> BTreeSet<String> {
        self.names(&lit_vars(self.lits[i]))
    }
    /// The head's variables, those inside a call too.
    fn head_vars(&self) -> BTreeSet<String> {
        let mut vs = VarSet::new();
        self.r.head().args().for_each(|t| term_vars(t, &mut vs));
        self.names(&vs)
    }
    fn rel(&self, a: AtomRef<'_>) -> &'v str {
        self.v.rel_name(a.rel())
    }
    fn head(&self) -> &'v str {
        self.rel(self.r.head())
    }
    /// Literal `i` as an atom (an aggregate's too).
    fn atom(&self, i: usize) -> Option<AtomRef<'v>> {
        self.lits[i].atom()
    }
    /// Literal `i` as a plain atom.
    fn plain(&self, i: usize) -> Option<AtomRef<'v>> {
        match self.lits[i].view() {
            Lit::Atom(a) => Some(a),
            _ => None,
        }
    }
}

/// A base relation of the corpus: an input, a view of an input's columns, a `nearest_<set>` or an `inside_<set>`.
fn base(f: &Fields<'_>, rel: &str, a: AtomRef<'_>) -> bool {
    f.is_input(rel) || rel.starts_with("nearest_") || rel.starts_with(super::super::super::front::AXIS) || view_of(f, a).is_some()
}

/// Atom `a` of a view of an input's columns (Q37, `kind(n, k) :- node(_, n, k, _, _)`) as the input's reads it: the
/// input, and its column of each of the view's. A view is the input's rows, so it is a base relation here: taken as
/// derived, `kind(o, "type_alias_statement")` bound nothing, a definition's `field(o, "left", l)` (8,152 rows in a
/// chunk of PostHog) made `tp_name` large and so demanded, and `tp_ref`'s split made every identifier with each of
/// its ancestors (55 M rows a cold run; work queue Q88's measurement).
fn view_of<'v>(f: &Fields<'v>, a: AtomRef<'_>) -> Option<(String, Vec<usize>)> {
    if f.is_input(f.v.rel_name(a.rel())) {
        return None;
    }
    let mut rules = f.v.heads(a.rel());
    let (Some(r), None) = (rules.next(), rules.next()) else { return None };
    super::super::input_view_shape(f.v, r, |x| f.is_input(x))
}

/// The rows a base atom reaches: its relation's, or, with constants, the fewest postings of one of them (a view's
/// those of its input's columns).
fn base_rows(f: &Fields<'_>, model: &super::super::Model, rel: &str, a: AtomRef<'_>) -> f64 {
    let view = view_of(f, a);
    let (rel, at): (&str, Option<&[usize]>) = match &view {
        Some((input, at)) => (input.as_str(), Some(at.as_slice())),
        None => (rel, None),
    };
    let all = if rel.starts_with("nearest_") || rel.starts_with(super::super::super::front::AXIS) { model.stats.nodes } else { model.card(rel) };
    let names: Option<Vec<&str>> = f.v.decl_of(rel).map(|d| d.cols().map(|(n, _)| f.v.text(n)).collect());
    a.args()
        .enumerate()
        .filter_map(|(c, t)| {
            let c = match at {
                Some(at) => *at.get(c)?,
                None => c,
            };
            let v = match t {
                TermRef::Int(i) => super::super::Constant::Int(i),
                TermRef::Branded(a) => super::super::Constant::Int(a.value()),
                _ => return None,
            };
            let name = *names.as_ref()?.get(c)?;
            // A value missing from estimated postings (`Stats::estimated`) is rare here: the demand's bar (`large`)
            // asks whether a scan is big, and a value the counted sample lacks is not, so it counts none, as before
            // estimates were marked. The join pricer takes such a value as unknown instead. Taken as the whole
            // relation, or as one value's share (this model has no distinct counts for the inputs yet: the whole
            // again), a rare field (`#f65567`, a definition's type parameters) made `tp_name` demanded, and
            // `tp_ref`'s split made every identifier with each of its ancestors (54 M rows of a cold PostHog run).
            model.stats.const_freq(rel, name, &v).map(|n| n as f64).or_else(|| model.stats.estimated.contains(rel).then_some(0.0))
        })
        .fold(all, f64::min)
}

/// An atom that restricts the rows a body reaches: a derived relation (not a text test, whose selectivity is unknown),
/// or a base atom whose measured rows are fewer than a scan's (`node(_, $n, 1223, _, _)`, not the 6M identifiers of
/// `node(_, $n, 1025, _, _)`).
fn anchor(f: &Fields<'_>, model: &super::super::Model, rel: &str, a: AtomRef<'_>) -> bool {
    if rel.starts_with("strtest_") || rel.starts_with("demand_") {
        return false;
    }
    !base(f, rel, a) || base_rows(f, model, rel, a) < model.stats.nodes / 32.0
}

/// The relations `split_negations` demands, before its rounds (the module's conditions): derived, not kept, not
/// recursive, read only by atoms of variable or constant arguments, and large (or `magic`).
pub(super) fn candidates<'v>(f: &Fields<'v>, p: &'v Placed, model: &super::super::Model) -> BTreeSet<&'v str> {
    let n = p.len();
    let rules: Vec<R<'v>> = (0..n).map(|k| R::of(p, k)).collect();
    let kept = |rel: &str| {
        f.is_output(rel)
            || f.is_input(rel)
            || f.lattice(rel)
            || f.native(rel).is_some()
            || f.has_facts(rel)
            || f.demanded(rel)
            || f.gate(rel)
            || f.qualified(rel, "no_magic")
    };
    let mut by_rel: BTreeMap<&str, Vec<&R<'v>>> = BTreeMap::new();
    for r in &rules {
        by_rel.entry(r.head()).or_default().push(r);
    }
    // Starts from every node: a `nearest_<set>`, `inside_<set>` or kind-free `node` atom on its key.
    let per_node = |r: &R<'_>| -> bool {
        let Some(TermRef::Var(key)) = r.r.head().args().next() else { return false };
        let on_key = |a: AtomRef<'_>, c: usize| a.args().nth(c).and_then(|t| t.var()) == Some(key);
        (0..r.lits.len()).any(|i| match r.lits[i].view() {
            Lit::Atom(a) if !a.neg() => {
                let rel = r.rel(a);
                ((rel.starts_with("nearest_") || rel.starts_with(super::super::super::front::AXIS)) && on_key(a, 0))
                    || (rel == "node" && on_key(a, 1) && matches!(a.args().nth(2), Some(TermRef::Wild)))
            }
            Lit::Nearest { n, .. } => n == key,
            _ => false,
        })
    };
    // A scan of the corpus: per head variable, the fewest rows of a base atom that binds it (measured counts of the
    // base relations and of the program's constants, not estimates of derived relations), and the most of those over
    // the head's variables; `None` when no base atom binds one. Not the key's alone: in
    // `restores(s, x, a) :- binding(s, x, site, _), site != a, earlier(site, a)` a derived atom binds the key `s`,
    // and `earlier`'s `docof(a, d)` makes a row for every node after each site (1.6 M rows on the Python pack's
    // fixtures, for a reader that asks about 483).
    let scan = |r: &R<'_>| -> Option<f64> {
        r.r.head()
            .args()
            .filter_map(|t| t.var())
            .filter_map(|v| {
                (0..r.lits.len())
                    .filter_map(|i| match r.lits[i].view() {
                        Lit::Atom(a) if !a.neg() && base(f, r.rel(a), a) && a.args().any(|t| t.var() == Some(v)) => Some(base_rows(f, model, r.rel(a), a)),
                        _ => None,
                    })
                    .reduce(f64::min)
            })
            .reduce(f64::max)
    };
    // A scan is worth demanding when it reaches at least this many rows: the rows not built must outweigh the strata
    // the split adds (`pre_<i>` and `demand_<r>`, opened in every chunk). Its other rules (a join of derived relations,
    // `m132(n) :- pre(n), m131(n)`) take the demand too, which only narrows them.
    let large = model.stats.nodes / 32.0;
    let demandable = |rs: &[&R<'_>]| rs.iter().all(|r| per_node(r)) || rs.iter().filter_map(|r| scan(r)).sum::<f64>() >= large;
    let recursive = |rel: &str, rs: &[&R<'_>]| rs.iter().any(|r| (0..r.lits.len()).any(|i| r.atom(i).is_some_and(|a| r.rel(a) == rel)));
    let reads_demand = |r: &R<'_>| (0..r.lits.len()).any(|i| r.plain(i).is_some_and(|a| r.rel(a).starts_with("demand_")));
    let mut candidates: BTreeSet<&'v str> = by_rel
        .iter()
        .filter(|(rel, rs)| !kept(rel) && !recursive(rel, rs) && rs.iter().all(|r| !reads_demand(r)) && (demandable(rs) || f.qualified(rel, "magic")))
        .map(|(rel, _)| *rel)
        .collect();
    // Read only by atoms (negated or positive) whose arguments are variables or constants; and read somewhere: a
    // relation nothing reads stays as it is.
    let mut read: BTreeSet<&str> = BTreeSet::new();
    for r in &rules {
        for i in 0..r.lits.len() {
            match r.lits[i].view() {
                Lit::Atom(a) if a.args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_))) => {}
                Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                    candidates.remove(r.rel(a));
                }
                _ => {}
            }
            if let Some(a) = r.atom(i) {
                read.insert(r.rel(a));
            }
        }
    }
    candidates.retain(|r| read.contains(r));
    candidates
}

/// Whether the positive atoms `cut` leaves fall apart: two of them share no variable through the others (or
/// through an equality).
fn disconnected(r: &R<'_>, cut: &dyn Fn(usize) -> bool) -> bool {
    // A document variable does not connect: a document has a row for each of its nodes, so atoms joined only by
    // their documents (`docof($s, $e9), rr_cx_reach($d9, $e9)` and a row of `$d9`'s) are a product per pair of
    // documents. 4,719 `anc` rows times the annotations of every document they reach made 24M rows (O136).
    let mut documents: BTreeSet<String> = BTreeSet::new();
    for i in 0..r.lits.len() {
        let Some(a) = r.atom(i).filter(|a| !a.neg()) else { continue };
        let rel = r.rel(a);
        let terms: Vec<TermRef<'_>> = match rel {
            "docof" => a.args().nth(1).into_iter().collect(),
            x if x.starts_with("rr_cx_") => a.args().collect(),
            _ => Vec::new(),
        };
        documents.extend(terms.into_iter().filter_map(|t| t.var()).map(|s| r.name(s).to_owned()));
    }
    let mut groups: Vec<BTreeSet<String>> = Vec::new();
    for j in 0..r.lits.len() {
        let joins = match r.lits[j].view() {
            Lit::Atom(a) => !a.neg() && !cut(j),
            Lit::Cmp { op: CmpOp::Eq, .. } => true,
            _ => false,
        };
        let vars: BTreeSet<String> = r.lit_vars(j).into_iter().filter(|v| !documents.contains(v)).collect();
        if !joins || vars.is_empty() {
            continue;
        }
        let (mut joined, rest): (Vec<_>, Vec<_>) = groups.into_iter().partition(|g| !g.is_disjoint(&vars));
        let mut all = vars;
        joined.iter_mut().for_each(|g| all.append(g));
        groups = rest;
        groups.push(all);
    }
    groups.len() > 1
}

/// Whether the body of `rule` less its demanded reads (`cut(j)`) is small: every kept positive literal is reached
/// from the kept anchors (and, where the reader is itself demanded, from its head) through lookups of bounded fan-out.
/// A base relation is read by a node it is keyed on: `node` by its id; `field` and `child` by the parent or the
/// child (a node's fields and children, its one parent); `doc` by its id or path; an axis or any other base
/// relation by its first column (a node's ancestors, its one nearest, its text). Structural, not statistics: no
/// estimate of a derived relation is read, and an axis read from its ancestor end (every descendant) is not a
/// lookup. Decision 0048's extension, since axes are made per looked-up node (decision 0057).
/// Also the variables so bound.
fn reached(f: &Fields<'_>, model: &super::super::Model, r: &R<'_>, own: bool, cut: &dyn Fn(usize) -> bool) -> (BTreeSet<String>, bool) {
    let mut bound: BTreeSet<String> = if own { r.head_vars() } else { BTreeSet::new() };
    let lookup = |a: AtomRef<'_>, bound: &BTreeSet<String>| {
        let on = |c: usize| match a.args().nth(c) {
            Some(TermRef::Var(s)) => bound.contains(r.name(s)),
            Some(TermRef::Wild) | None => false,
            Some(_) => true,
        };
        match r.rel(a) {
            "node" => on(1),
            "field" => on(0) || on(2),
            "child" => on(0) || on(1),
            "doc" => on(0) || on(1),
            _ => on(0),
        }
    };
    let mut todo: Vec<usize> = (0..r.lits.len())
        .filter(|&j| match r.lits[j].view() {
            Lit::Atom(b) => !b.neg() && !cut(j),
            Lit::Nearest { .. } => true,
            _ => false,
        })
        .collect();
    loop {
        let before = todo.len();
        todo.retain(|&j| {
            let hit = match r.lits[j].view() {
                Lit::Atom(b) => anchor(f, model, r.rel(b), b) || lookup(b, &bound),
                Lit::Nearest { n, .. } => bound.contains(r.name(n)),
                _ => true,
            };
            if hit {
                bound.extend(r.lit_vars(j));
            }
            !hit
        });
        if todo.is_empty() || todo.len() == before {
            return (bound, todo.is_empty());
        }
    }
}

/// What a round of the split decides (`split_with`'s readings): each reader's split, the declarations of its
/// `pre_<i>` (its columns and types), and the nullary ones; or the candidates refused this round.
pub(super) type Analysis = (BTreeMap<usize, Split>, Vec<(String, Vec<(String, String)>)>, Vec<String>);

/// `split_with`'s readings, over the version: the refusals, then each reader's split.
pub(super) fn analyse(f: &Fields<'_>, p: &Placed, model: &super::super::Model, candidates: &BTreeSet<&str>) -> Result<Analysis, BTreeSet<String>> {
    let n = p.len();
    let rules: Vec<R<'_>> = (0..n).map(|k| R::of(p, k)).collect();
    let negated = |r: &R<'_>, i: usize| r.plain(i).is_some_and(|a| candidates.contains(r.rel(a)));
    let readers: Vec<usize> = (0..n).filter(|&k| (0..rules[k].lits.len()).any(|i| negated(&rules[k], i))).collect();
    // A positive read is cut only where one kept anchor binds all its variables (the module's guard). Every reader is
    // checked before any is split, so one round drops every candidate a guard refuses.
    let mut refused = BTreeSet::new();
    for &k in &readers {
        let r = &rules[k];
        let cut = |j: usize| negated(r, j);
        // A reader with nothing but its demanded reads (`p :- !allow_status("plan_helpers", "valid")`), and no demand
        // of its own: no `pre_<i>` binds it, so it would stay as written and demand nothing, while its reads' relations
        // are computed only for their other readers' keys. Its keys would hold no rows, and each negation would hold
        // (a constant finding reported where its status was valid). Its reads' relations are built whole.
        if !candidates.contains(r.head()) && (0..r.lits.len()).all(|i| negated(r, i)) {
            refused.extend((0..r.lits.len()).filter_map(|i| r.atom(i)).map(|a| r.rel(a).to_owned()));
            continue;
        }
        for i in 0..r.lits.len() {
            let Some(a) = r.plain(i) else { continue };
            if a.neg() || !negated(r, i) {
                continue;
            }
            let vars = r.lit_vars(i);
            let anchors = |demanded: bool| {
                (0..r.lits.len()).find_map(|j| match r.plain(j) {
                    Some(b) if j != i && !b.neg() && negated(r, j) == demanded && anchor(f, model, r.rel(b), b) && vars.is_subset(&r.lit_vars(j)) => Some(r.rel(b)),
                    _ => None,
                })
            };
            // Bound by the kept anchors through bounded lookups; or anchored only by another demanded read, which then
            // stays in the body instead (its relation is built whole).
            let own = candidates.contains(r.head());
            let (reach, small) = reached(f, model, r, own, &cut);
            if anchors(false).is_none() && !vars.is_subset(&reach) {
                match anchors(true) {
                    Some(b) => refused.insert(b.to_owned()),
                    None => refused.insert(r.rel(a).to_owned()),
                };
                continue;
            }
            // The body less the read must be small too: every kept positive atom reached from the anchors by a bounded
            // lookup (`reached`; an `inside_<set>` read from its ancestor, a rare kind, still reaches 18M nodes),
            // unless the reader is itself demanded, when its `pre_<i>` starts from that demand.
            if !own && !small {
                refused.insert(r.rel(a).to_owned());
            }
            // The body less the demanded reads must be one join: kept atoms that only a demanded read connects
            // (`p13_er_body(f, n)` and `p13_unparen(e, m)`, joined by the read alone) would make `pre_<i>` their
            // cross product.
            if !own && disconnected(r, &cut) {
                refused.insert(r.rel(a).to_owned());
            }
        }
    }
    if !refused.is_empty() {
        super::super::super::profile::decision("demand (decision 0048)", || format!("not demanded: {} (read positively where no anchor binds the read, or the rest of the body is not small)", refused.iter().cloned().collect::<Vec<_>>().join(", ")));
        return Err(refused);
    }
    // Every rule whose split has a variable of no known type, found in one pass (one round each made 126 rounds of
    // the whole program's split and strata on the Python pack).
    let mut untyped: BTreeSet<String> = BTreeSet::new();
    let (mut splits, mut decls, mut nullary) = (BTreeMap::new(), Vec::new(), Vec::new());
    for &k in &readers {
        let r = &rules[k];
        let cut: Vec<usize> = (0..r.lits.len()).filter(|&i| negated(r, i)).collect();
        let kept: Vec<usize> = (0..r.lits.len()).filter(|i| !cut.contains(i)).collect();
        let own = candidates.contains(r.head());
        // The reader's own demand's variables (its head's, a computed column a fresh `rr_demanded<i>`, which a
        // comparison binds with the column's variables), and the type a variable takes from the first positive
        // atom of its kept literals that names it, its own demand first (`var_type`).
        let mut bound: BTreeSet<String> = BTreeSet::new();
        let mut own_atom: Vec<(Option<String>, Option<(String, usize)>)> = Vec::new();
        if own {
            let demand = format!("demand_{}", r.head());
            for (c, t) in r.r.head().args().enumerate() {
                match t {
                    TermRef::Call(..) => {
                        bound.insert(format!("rr_demanded{c}"));
                        let mut vs = VarSet::new();
                        term_vars(t, &mut vs);
                        bound.extend(r.names(&vs));
                    }
                    TermRef::Var(s) => {
                        bound.insert(r.name(s).to_owned());
                        own_atom.push((Some(r.name(s).to_owned()), Some((demand.clone(), c))));
                    }
                    _ => {}
                }
            }
        }
        // Nothing left to bind the demand (every literal is a negated read): a `pre_<i>` of an empty body is no
        // relation any engine makes, so the rule stays as it is.
        if !own && kept.is_empty() {
            continue;
        }
        for &i in &kept {
            bound.extend(r.lit_vars(i));
        }
        let mut later = r.head_vars();
        for &i in &cut {
            later.extend(r.lit_vars(i));
        }
        let vars: Vec<String> = bound.intersection(&later).cloned().collect();
        let var_type = |var: &str| -> Option<String> {
            // The own demand's atom first: its relation is declared only once demanded (`split_with`), so a type is
            // taken from it only where the program declares it already.
            if let Some((_, Some((rel, c)))) = own_atom.iter().find(|(v, _)| v.as_deref() == Some(var))
                && let Some(t) = f.v.decl_of(rel).and_then(|d| d.cols().nth(*c)).map(|(_, t)| f.v.text(t).to_owned()) {
                    return Some(t);
                }
            kept.iter().find_map(|&i| match r.lits[i].view() {
                Lit::Atom(a) if !a.neg() => {
                    let c = a.args().position(|t| t.var().is_some_and(|s| r.name(s) == var))?;
                    f.v.decl_of(r.rel(a))?.cols().nth(c).map(|(_, t)| f.v.text(t).to_owned())
                }
                Lit::Nearest { n, a, .. } if r.name(n) == var || r.name(a) == var => Some("node".into()),
                _ => None,
            })
        };
        let Some(types) = vars.iter().map(|v| var_type(v)).collect::<Option<Vec<String>>>() else {
            super::super::super::profile::decision("demand (decision 0048)", || format!("not demanded: a variable of a rule for {} has no known type", r.head()));
            untyped.extend(cut.iter().filter_map(|&i| r.atom(i)).map(|a| r.rel(a).to_owned()));
            continue;
        };
        let pre = format!("pre_{k}");
        let mut cols: Vec<(String, String)> = vars.iter().cloned().zip(types).collect();
        // A split sharing no variable with its reads is a flag, encoded as every nullary relation is: one unit column
        // (`rr::lower::lower_boolean_relations`).
        if vars.is_empty() {
            nullary.push(pre.clone());
            cols.push(("rr_unit".to_owned(), "int".to_owned()));
        }
        decls.push((pre.clone(), cols));
        splits.insert(k, Split { pre, vars, cut: cut.into_iter().collect() });
    }
    if !untyped.is_empty() {
        return Err(untyped);
    }
    Ok((splits, decls, nullary))
}

#[cfg(test)]
mod tests {
    use super::{R, base, disconnected, view_of};
    use super::super::super::body::Placed;
    use super::super::super::fields::Fields;

    /// Whether rule `k` of `text` falls apart once its reads of `cut` are cut.
    fn apart(text: &str, k: usize, cut: &str) -> bool {
        let prog = super::super::super::super::front::parse(text).unwrap();
        let p = Placed::of_program(&prog).unwrap();
        let r = R::of(&p, k);
        disconnected(&r, &|j| r.plain(j).is_some_and(|a| r.rel(a) == cut))
    }

    /// A split whose kept atoms only the demanded read joins is refused: its `pre_<i>` would be their cross product.
    #[test]
    fn kept_atoms_that_only_the_cut_joins_are_disconnected() {
        let p = r#"
            .decl body(f: node, n: node)
            .decl unp(e: node, m: node)
            .decl d(n: node, e: node)
            .decl out(f: node, e: node)
            .input body, unp, d
            .output out
            out($f, $e) :- body($f, $n), unp($e, $m), d($n, $e).
            out($f, $e) :- body($f, $n), unp($e, $n), d($n, $e).
            "#;
        assert!(apart(p, 0, "d"), "joined by the read alone");
        assert!(!apart(p, 1, "d"), "joined by n");
    }

    /// Atoms joined only through their documents are apart (O136): cutting `uinst` would leave every annotation of
    /// every reached document against every `anc` row.
    #[test]
    fn atoms_joined_only_by_their_documents_are_disconnected() {
        let p = r#"
            .decl ann(f: node, x: number, t: node)
            .decl uinst(t: node, c: node)
            .decl anc(s: node, c: node)
            .decl docof(n: node, d: number)
            .decl rr_cx_reach(d: number, e: number)
            .decl bv(f: node, x: number, s: node)
            .input ann, uinst, anc, docof, rr_cx_reach
            .output bv
            bv($f, $x, $s) :- ann($f, $x, $t), uinst($t, $c), anc($s, $c), docof($f, $d9), docof($s, $e9), rr_cx_reach($d9, $e9).
            "#;
        assert!(apart(p, 0, "uinst"), "cutting uinst leaves a product by documents");
        assert!(!apart(p, 0, "rr_cx_reach"), "the rest still joins by t and c");
    }

    /// A view of an input's columns (Q37) is a base relation to the split, read as its input's columns: `kind(o, 7)`
    /// is `node(_, o, 7, _, _)`, whose postings bound the scan. Taken as derived, a definition's `tp_name` was large by
    /// its `field` atom alone, and demanded (work queue Q88's measurement).
    #[test]
    fn a_view_of_an_input_reads_as_the_inputs_columns() {
        let p = r#"
            .decl node(d: number, n: node, kind: number, s: number, e: number)
            .decl field(p: node, f: number, c: node)
            .decl kind(n: node, k: number)
            .decl named(o: node, c: node)
            .decl out(o: node)
            .input node, field
            .output out
            kind($n, $k) :- node(_, $n, $k, _, _).
            named($o, $c) :- kind($o, 7), field($o, 3, $c).
            out($o) :- named($o, _).
            "#;
        let prog = super::super::super::super::front::parse(p).unwrap();
        let placed = Placed::of_program(&prog).unwrap();
        let f = Fields::of(&placed).unwrap();
        let k = (0..placed.len()).find(|&k| R::of(&placed, k).head() == "named").unwrap();
        let r = R::of(&placed, k);
        let atom = |rel: &str| (0..r.lits.len()).find_map(|i| r.plain(i).filter(|a| r.rel(*a) == rel)).unwrap();
        assert_eq!(view_of(&f, atom("kind")), Some(("node".to_owned(), vec![1, 2])), "kind's columns are node's 1 and 2");
        assert!(base(&f, "kind", atom("kind")), "a view of an input is a base relation");
        assert_eq!(view_of(&f, atom("field")), None, "an input is no view");
        let n = (0..placed.len()).find(|&k| R::of(&placed, k).head() == "out").unwrap();
        let o = R::of(&placed, n);
        let named = (0..o.lits.len()).find_map(|i| o.plain(i)).unwrap();
        assert!(!base(&f, "named", named), "a derived relation is not");
    }
}
