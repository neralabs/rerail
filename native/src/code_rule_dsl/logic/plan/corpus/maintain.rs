//! A session's changed strata maintain their rows from deltas (decision 0077 §3, stage 2): delete-and-rederive,
//! written as rules the native engine runs as it runs any program. One program, one evaluation, no second engine.
//!
//! For a changed relation `r` (an input whose rows changed, or a relation of a stratum that runs) the program has its
//! new rows under its own name, its old rows as input `r__old` (from what the last run kept), and its deltas:
//!
//! ```text
//! r__minus(x) :- r__old(x), !r(x).        rows gone
//! r__plus(x)  :- r(x), !r__old(x).        rows new
//! ```
//!
//! A stratum `S` whose rules are joins, negations and comparisons is maintained. For each rule `h :- B` and each
//! position `k` of `B` that can change (an atom of `S`, or of a changed relation):
//!
//! ```text
//! overdelete  h__del(..) :- B over old rows, k as its loss     (q__del, r__minus; a negated !n as n__plus)
//! survivors   h(..)      :- h__old(..), !h__del(..).
//! rederive    h__redo(..) :- h__del(..), B over new rows.        h(..) :- h__redo(..).
//! insert      h__ins(..) :- B over new rows, k as its gain     (q__ins, r__plus; a negated !n as n__minus, !n)
//!             h(..)      :- h__ins(..).
//! ```
//!
//! A maintained relation's net deltas, for the strata after it, are made from those three, each a delta's size: a
//! row gone was overdeleted and neither rederived nor inserted, a row new was inserted and not there before.
//!
//! ```text
//! h__minus(..) :- h__del(..), !h__redo(..), !h__ins(..).
//! h__plus(..)  :- h__ins(..), !h__old(..).
//! ```
//!
//! An input's, or a stratum's derived again, are a diff of its old and new rows (`r__minus`, `r__plus` above).
//!
//! An overdeleted row is a superset of the rows that lost every derivation; a survivor holds in the new rows. A new
//! row not among the old has a derivation using a gained row at some position, by induction on its derivation, so
//! the insert rules reach it. A head's computed term (`@dot(p, s)`) is first named: `h(@dot(p, s)) :- B` is read as
//! `h(v) :- B, v = @dot(p, s)`, so the rows above are rows of variables. A `nearest` literal reads the tree (`node`,
//! `child`): while neither changed it is copied as a comparison is, and once either did its stratum is derived
//! again.
//!
//! An aggregate over a changed relation `a` is a position too, by its groups: the atom's correlated variables `g`
//! over `a`'s rows gone and new are the groups whose value may have changed.
//!
//! ```text
//! grp(g)      :- a__minus(..).   grp(g) :- a__plus(..).
//! overdelete  h__del(..) :- grp(g), B over old rows (the aggregate over a__old)
//! insert      h__ins(..) :- grp(g), B over new rows
//! ```
//!
//! A head row that read an unchanged group read the same value before and after, so the other positions' rules cover
//! it (with the aggregate over old rows in an overdelete). A stratum with a relation made natively, a lattice, an
//! `ast` literal, or a changed read whose old rows were not kept, is derived again from its inputs: its rules
//! unchanged.
//! Either way its relations' deltas feed the strata after it, and an empty delta derives nothing there.

use super::retain::{Cut, Retained};
use super::{Error, Result, lower};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The part as a run evaluates it, the old rows it reads, and how its strata were evaluated.
pub(in crate::code_rule_dsl) struct Maintained {
    pub(in crate::code_rule_dsl) program: lower::Lowered,
    /// `r__old` for each changed relation read, from what was kept: a source, and for a derived relation the
    /// engine's relation, adopted by the caller (read in place).
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>,
    /// Strata maintained from deltas, and derived again.
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Of those maintained, strata that are recursive.
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Each stratum derived again, by one of its relations, and why.
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
    /// The relations that are views of one input (`input_views`): never kept, their old rows and deltas the input's.
    pub(in crate::code_rule_dsl) views: BTreeSet<String>,
}

/// The derived relations that are views of one input: one rule, one positive atom of an input, every column of the
/// input in the head (a variable the head names, or a constant), so a row of it is one row of the input. With
/// `RERAIL_SESSION_VIEWS=on` a session keeps such a relation as a view (decision 0090): its old rows are the view
/// over the input's old rows, its delta the view over the input's delta, and none of its rows are kept.
///
/// By default none is: a native run makes no views (decision 0104), so a view's stratum copied its input twice
/// each run, as itself and as its old rows (`kind__sup0` and `kind__sup0__old` each a scan of `node`'s 243,092
/// rows, 0.8 s of an O125 step for four such views). Kept and maintained as other relations, a view costs its
/// delta, and its rows' memory (about 10 MB each there).
pub(in crate::code_rule_dsl) fn input_views(prog: &front::Program) -> BTreeMap<String, Rule> {
    if !std::env::var("RERAIL_SESSION_VIEWS").is_ok_and(|v| v == "on") {
        return BTreeMap::new();
    }
    let mut by: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for r in &prog.rules {
        by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    by.into_iter()
        .filter_map(|(rel, rules)| {
            let [rule] = rules.as_slice() else { return None };
            let [Literal::Atom(a)] = rule.body.as_slice() else { return None };
            if a.neg || a.gated || !prog.inputs.iter().any(|x| x == &a.rel) || prog.lattice.contains_key(rel) || prog.natives.contains_key(rel) || prog.facts.contains_key(rel) || prog.outputs.iter().any(|o| o == rel) {
                return None;
            }
            let head: BTreeSet<&str> = rule.head.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.as_str()) } else { None }).collect();
            let terms_ok = rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_)));
            let one_to_one = a.args.iter().all(|t| match t {
                Term::Var(v) => head.contains(v.as_str()),
                Term::Int(_) | Term::Str(_) => true,
                _ => false,
            });
            (terms_ok && one_to_one).then(|| (rel.to_owned(), (*rule).clone()))
        })
        .collect()
}

fn named(rel: &str, suffix: &str) -> String {
    format!("{rel}__{suffix}")
}

/// An insert rule of `h`, carrying only rows new against the survivors (`h__old` and not `h__del`): an insert
/// stops where a row survived, as semi-naive evaluation stops at a row already held. Without it one new edge
/// derived a recursive relation's whole closure again (9,000,000 inserted rows for 2 new, `session_costs`).
/// Not a survivor is not old, or deleted: two rules, each a lower stratum's read. A head with a call keeps the
/// rule unguarded.
fn insert(rules: &mut Vec<Rule>, h: &str, args: &[Term], body: Vec<Literal>, gone: &str) {
    let head = Atom { rel: named(h, "ins").into(), args: args.to_vec(), neg: false, gated: false };
    if !args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) {
        rules.push(Rule { head, body });
        return;
    }
    let mut new = body.clone();
    new.push(atom(named(h, "old"), args.to_vec(), true));
    rules.push(Rule { head: head.clone(), body: new });
    let mut deleted = body;
    deleted.push(atom(named(h, gone), args.to_vec(), false));
    rules.push(Rule { head, body: deleted });
}

/// Whether recursive strata are maintained by levels (decision 0090), not delete-and-rederive.
pub(in crate::code_rule_dsl) fn levels_wanted() -> bool {
    // The default (decision 0090, levels checked); `RERAIL_RECURSIVE=dred` keeps delete-and-rederive.
    std::env::var("RERAIL_RECURSIVE").map_or(true, |v| v != "dred")
}

fn atom(rel: String, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel: rel.into(), args, neg, gated: false })
}

fn row(width: usize) -> Vec<Term> {
    (0..width).map(|i| Term::Var(format!("_m{i}").into())).collect()
}

/// Why a stratum's rules cannot be maintained, if they cannot: joins, negations, comparisons, `nearest` and aggregates over
/// earlier strata only, heads of variables and constants (after `named_heads`), and every read that changed has its old rows.
fn unmaintainable(prog: &front::Program, scc: &BTreeSet<String>, rules: &[&Rule], changed: &BTreeSet<String>, old: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    for r in scc {
        if prog.natives.contains_key(r) {
            return Some("a relation made natively");
        }
        if prog.lattice.contains_key(r) {
            return Some("a lattice");
        }
        if !old(r) {
            return Some("its own old rows not kept");
        }
    }
    // Counting (`supports`) is for non-recursive strata: a recursive one keeps its rules and is maintained below by
    // levels, or by delete-and-rederive (`RERAIL_COUNT_RECURSIVE=again`: derived again, as the first comparison did).
    if super::supports::counting()
        && std::env::var("RERAIL_COUNT_RECURSIVE").is_ok_and(|v| v == "again")
        && rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(a.rel.as_str()))))
    {
        return Some("recursive, under counting");
    }
    for rule in rules {
        if !rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) {
            return Some("a head term");
        }
        for l in &rule.body {
            match l {
                Literal::Atom(a) if a.gated => return Some("a gated negation"),
                Literal::Atom(a) if changed.contains(a.rel.as_str()) && !scc.contains(a.rel.as_str()) && !old(&a.rel) => return Some("a changed read's old rows not kept"),
                Literal::Nearest { .. } if super::retain::TREE.iter().any(|r| changed.contains(*r)) => return Some("a nearest literal over a changed tree"),
                Literal::Atom(_) | Literal::Cmp { .. } | Literal::Nearest { .. } => {}
                // An aggregate over an earlier stratum's relation (never this one's: aggregation is not recursive).
                Literal::Agg { atom: a, .. } if a.gated => return Some("a gated negation"),
                Literal::Agg { atom: a, .. } if scc.contains(a.rel.as_str()) => return Some("an aggregate over its own stratum"),
                Literal::Agg { atom: a, .. } if a.args.iter().any(|t| matches!(t, Term::Call(..))) => return Some("an aggregate over a computed term"),
                Literal::Agg { atom: a, .. } if changed.contains(a.rel.as_str()) && !old(&a.rel) => return Some("a changed read's old rows not kept"),
                Literal::Agg { .. } => {}
                Literal::Ast { .. } => return Some("an ast literal"),
            }
        }
    }
    None
}

/// `rule` with each computed head term named by a variable the body binds: `h(@f(x)) :- B` as `h(v) :- B, v = @f(x)`.
fn named_heads(rule: &Rule) -> Rule {
    let mut rule = rule.clone();
    for (i, t) in rule.head.args.iter_mut().enumerate() {
        if matches!(t, Term::Call(..)) {
            let v = Term::Var(format!("_h{i}").into());
            let call = std::mem::replace(t, v.clone());
            rule.body.push(Literal::Cmp { l: v, op: "=", r: call });
        }
    }
    rule
}

/// `part` (the cut's program) with each changed stratum maintained where it can be, and the old rows it reads.
/// `given` answers for an input whose delta it has (its rows new and gone, found by its documents), bound as
/// `r__plus` and `r__minus` in place of their diff of every old and new row; asked only for the inputs a maintained
/// stratum reads the delta of.
/// One stratum's form, made when a run reaches it (decision 0117 step 3): `rewrite` over a program of that stratum
/// alone. Its relations' deltas are always made, since the run asks whether they are empty; a delta or an old
/// source that is a relation of the run already (an earlier stratum's form made or bound it) is read, not made again.
pub(in crate::code_rule_dsl) struct One<'m> {
    pub(in crate::code_rule_dsl) has_delta: &'m dyn Fn(&str) -> bool,
    pub(in crate::code_rule_dsl) has_old: &'m dyn Fn(&str) -> bool,
}

/// `sccs`: `cross`'s strata, computed once by the caller (as `retain::cut` was given them).
pub(in crate::code_rule_dsl) fn rewrite(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered, sccs: &[BTreeSet<String>], given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>) -> Result<Maintained> {
    rewrite_with(cut, kept, cross, sccs, given, None)
}

pub(in crate::code_rule_dsl) fn rewrite_with(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered, sccs: &[BTreeSet<String>], given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>, one: Option<&One>) -> Result<Maintained> {
    let mut part = cut.program(cross);
    let dirty: Vec<&BTreeSet<String>> = sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).collect();
    let why = if kept.is_none() { "nothing kept" } else { "the program has gates" };
    let Some(kept) = kept.filter(|_| cross.program.gates.is_empty()) else {
        let again = dirty.iter().map(|scc| (scc.iter().next().cloned().unwrap_or_default(), why)).collect();
        let views = input_views(&cross.program).into_keys().collect();
        return Ok(Maintained { program: part, sources: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0, again, views });
    };
    // The views of one input whose input's old rows are kept: what they read is the input's (`input_views`).
    let views: BTreeMap<String, Rule> = input_views(&cross.program).into_iter().filter(|(_, r)| r.body[0].atom().is_some_and(|a| kept.has_old(&a.rel))).collect();
    let old = |rel: &str| kept.has_old(rel) || views.contains_key(rel);
    let mut changed: BTreeSet<String> = cut.changed.iter().cloned().collect();
    let mut rules: Vec<Rule> = Vec::new();
    let mut olds: BTreeSet<String> = BTreeSet::new();
    let mut deltas: BTreeSet<String> = BTreeSet::new();
    let (mut maintained, mut derived_again, mut recursive) = (0, 0, 0);
    let mut again: Vec<(String, &'static str)> = Vec::new();
    let mut fresh: Vec<(String, String)> = Vec::new();
    // Relations of the strata maintained: their net deltas come from their own del, redo and ins.
    let mut maint: BTreeSet<String> = BTreeSet::new();
    // Relations of strata maintained by levels: their rows gone are `h__gone`, not `h__del`.
    let mut gone_of: BTreeSet<String> = BTreeSet::new();
    // Of those, the ones maintained exactly (counting): their deletes are final, nothing is derived again.
    let mut exact_rels: BTreeSet<String> = BTreeSet::new();
    // Each maintained stratum, the span of `rules` its maintained form wrote, and its rules as given: its form
    // derived again (`front::Again`).
    let mut groups: Vec<(BTreeSet<String>, usize, usize, Vec<Rule>)> = Vec::new();
    // Each relation's rules, by position, found once: a scan of every rule per stratum (and per relation, to name one)
    // was 5% of a one-file run of the Python pack (16 k rules, 10 k strata).
    let mut by_head: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in part.program.rules.iter().enumerate() {
        by_head.entry(r.head.rel.as_str()).or_default().push(i);
    }
    let count_of = |rel: &str| by_head.get(rel).map_or(0, Vec::len);
    for scc in dirty {
        let mut at: Vec<usize> = scc.iter().filter_map(|r| by_head.get(r.as_str())).flatten().copied().collect();
        at.sort_unstable();
        let given: Vec<&Rule> = at.iter().map(|&i| &part.program.rules[i]).collect();
        // A view of one input stays one: its rule as given, changed when its input changed.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(view) = views.get(*rel)
        {
            rules.push(view.clone());
            if view.body[0].atom().is_some_and(|a| changed.contains(a.rel.as_str())) {
                changed.insert((*rel).clone());
            }
            continue;
        }
        let named_rules: Vec<Rule> = given.iter().map(|r| named_heads(r)).collect();
        let mine: Vec<&Rule> = named_rules.iter().collect();
        // `RERAIL_CROSS_MAINTAIN_ONLY`: maintain only the strata holding one of these relations (comma-separated; empty:
        // none), to find a stratum whose maintenance departs from deriving it again.
        let only = std::env::var("RERAIL_CROSS_MAINTAIN_ONLY").ok().map(|v| v.split(',').filter(|x| !x.is_empty()).map(str::to_owned).collect::<BTreeSet<String>>());
        let why = if only.as_ref().is_some_and(|o| !scc.iter().any(|r| o.contains(r))) { Some("not chosen") } else { unmaintainable(&part.program, scc, &mine, &changed, &old) };
        if std::env::var_os("O125_LIST").is_some() && why.is_none() {
            eprintln!("O125_LIST {}", scc.iter().next().unwrap());
        }
        if let Some(why) = why {
            let name = scc.iter().max_by_key(|r| count_of(r)).cloned().unwrap_or_default();
            again.push((format!("{name} ({} relations)", scc.len()), why));
            derived_again += 1;
            rules.extend(given.into_iter().cloned());
            changed.extend(scc.iter().cloned());
            continue;
        }
        maintained += 1;
        let start = rules.len();
        let as_given: Vec<Rule> = given.iter().map(|r| (*r).clone()).collect();
        let exact = scc.iter().all(|r| super::supports::is_support(r));
        // Under counting, a head whose every rule projects one of its supports (`h(x) :- h__sup0(x, _)`) is exact
        // too: a row is gone when it lost a support and none is left in the new rows, a count reaching zero. Its
        // rows that keep a support are neither deleted nor derived again, so they stay where they were.
        let projection = !exact
            && scc.len() == 1
            && mine.iter().all(|r| matches!(r.body.as_slice(), [Literal::Atom(a)] if !a.neg && super::supports::is_support(&a.rel)));
        // The rows that lost a derivation: deleted, or (a projection) checked for a support left first.
        let lost = |h: &str| if projection { named(h, "lost") } else { named(h, "del") };
        let is_recursive = mine.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(a.rel.as_str()))));
        if is_recursive {
            recursive += 1;
        }
        // Decision 0090: a recursive stratum's rows gone are decided in level order (`levels`) rather than
        // overdeleted, when its rules are joins, negations and comparisons and every kept row has its level.
        let lv = is_recursive && levels_wanted() && mine.iter().all(|r| r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. })))
            && scc.iter().all(|r| kept.kept(r).is_some_and(|(h, _)| h.has_levels()));
        if lv {
            BY_LEVELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if is_recursive && levels_wanted() && std::env::var_os("RERAIL_LEVELS_WHY").is_some() {
            let not_join = mine.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).count();
            let no_levels: Vec<(&String, (usize, usize, bool))> = scc.iter().filter_map(|r| kept.kept(r).filter(|(h, _)| !h.has_levels()).map(|(h, _)| (r, h.levels_shape()))).take(4).collect();
            let unkept: Vec<&String> = scc.iter().filter(|r| kept.kept(r).is_none()).take(4).collect();
            eprintln!("RERAIL_LEVELS_WHY {:?} ({} relations): levels {lv}; rules not joins {not_join}; without levels {no_levels:?}; not kept {unkept:?}", scc.iter().next(), scc.len());
            // The literals that are not joins, by kind, with a few of their rules.
            let kind = |l: &Literal| match l {
                Literal::Atom(_) | Literal::Cmp { .. } => None,
                Literal::Agg { .. } => Some("aggregate"),
                other => Some(match format!("{other:?}").split(['(', ' ', '{']).next() { Some("Assign") => "assignment", Some("Nearest") => "nearest", Some("Ast") => "ast", _ => "other" }),
            };
            let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
            for r in &mine {
                for l in &r.body {
                    if let Some(k) = kind(l) {
                        let shown = if k == "other" { format!("other: {}", format!("{l:?}").split(['(', ' ', '{']).next().unwrap_or("")) } else { k.to_owned() };
                        *kinds.entry(shown).or_default() += 1;
                    }
                }
            }
            eprintln!("RERAIL_LEVELS_WHY   literals not joins, by kind: {kinds:?}");
            for r in mine.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).take(6) {
                eprintln!("RERAIL_LEVELS_WHY   {r}");
            }
        }
        let gone = if lv { "gone" } else { "del" };
        // A read that can change: an atom of this stratum, or a changed relation (positive or negated).
        let moves = |a: &Atom| scc.contains(a.rel.as_str()) || changed.contains(a.rel.as_str());
        let old_of = |a: &Atom| if moves(a) { named(&a.rel, "old") } else { a.rel.to_string() };
        // A literal over old rows: an atom, or an aggregate's atom, that moves reads `r__old`.
        let old_lit = |l: &Literal| match l {
            Literal::Atom(b) => atom(old_of(b), b.args.clone(), b.neg),
            Literal::Agg { var, op, over, atom: b } => {
                Literal::Agg { var: var.clone(), op: op.clone(), over: over.clone(), atom: Atom { rel: old_of(b).into(), ..b.clone() } }
            }
            other => other.clone(),
        };
        for (r, rule) in mine.iter().enumerate() {
            let h = &rule.head.rel;
            for (k, lit) in rule.body.iter().enumerate() {
                if let Literal::Agg { var, op, over, atom: at } = lit
                    && moves(at)
                {
                    // The groups whose aggregate changed: the atom's correlated variables (all but the aggregated
                    // ones, as `plan::ready` reads them) over its rows gone and new. A row of the head that read one
                    // is deleted with the old value and inserted with the new; the other groups' values held.
                    let mut g = at.vars();
                    g.remove(var);
                    if let Some(o) = over {
                        g.remove(o);
                    }
                    if let front::AggOp::Join { by, .. } = op {
                        by.iter().for_each(|b| {
                            g.remove(b);
                        });
                    }
                    let grp = format!("{h}__grp{r}_{k}");
                    let cols = &part.program.decls[at.rel.as_str()];
                    let mut decl: Vec<(crate::code_rule_dsl::logic::front::Name, crate::code_rule_dsl::logic::front::Name)> = g
                        .iter()
                        .map(|v| {
                            let c = at.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).expect("a group variable is the atom's");
                            (v.into(), cols[c].1.clone())
                        })
                        .collect();
                    let mut key: Vec<Term> = g.iter().map(|v| Term::Var(v.into())).collect();
                    if decl.is_empty() {
                        decl.push(("rr_unit".into(), "int".into()));
                        key.push(Term::Int(1));
                        part.program.nullary.insert(grp.clone());
                    }
                    part.program.decls.insert(grp.clone().into(), decl.iter().map(|(c, t)| (c.into(), t.into())).collect());
                    for d in ["minus", "plus"] {
                        rules.push(Rule { head: Atom { rel: grp.clone().into(), args: key.clone(), neg: false, gated: false }, body: vec![atom(named(&at.rel, d), at.args.clone(), false)] });
                    }
                    deltas.insert(at.rel.to_string());
                    let mut over_old = vec![atom(grp.clone(), key.clone(), false)];
                    over_old.extend(rule.body.iter().map(old_lit));
                    rules.push(Rule { head: Atom { rel: named(h, "del").into(), args: rule.head.args.clone(), neg: false, gated: false }, body: over_old });
                    let mut over_new = vec![atom(grp, key, false)];
                    over_new.extend(rule.body.iter().cloned());
                    insert(&mut rules, h, &rule.head.args, over_new, "del");
                    continue;
                }
                let Literal::Atom(at) = lit else { continue };
                if !moves(at) {
                    continue;
                }
                let (lose, gain) = match (at.neg, scc.contains(at.rel.as_str())) {
                    (false, true) => (named(&at.rel, "del"), named(&at.rel, "ins")),
                    (false, false) => (named(&at.rel, "minus"), named(&at.rel, "plus")),
                    (true, _) => (named(&at.rel, "plus"), named(&at.rel, "minus")),
                };
                if !scc.contains(at.rel.as_str()) {
                    deltas.insert(at.rel.to_string());
                }
                // Under levels a premise of the stratum is lost when it is gone, and the head is a candidate.
                let lose = if lv && !at.neg && scc.contains(at.rel.as_str()) { named(&at.rel, "gone") } else { lose };
                let over: Vec<Literal> = rule
                    .body
                    .iter()
                    .enumerate()
                    .map(|(j, l)| match l {
                        Literal::Atom(b) if j == k => atom(lose.clone(), b.args.clone(), false),
                        other => old_lit(other),
                    })
                    .collect();
                rules.push(Rule { head: Atom { rel: if lv { named(h, "cand").into() } else { lost(h).into() }, args: rule.head.args.clone(), neg: false, gated: false }, body: over });
                // A negation's gain (`n__minus`) is a row gone, which makes `!n(..)` hold only when no other row
                // matches: with a wildcard (`!found(c, a, _)`) another may. The negation is kept, over the new rows.
                let ins: Vec<Literal> = rule
                    .body
                    .iter()
                    .enumerate()
                    .flat_map(|(j, l)| match j == k {
                        true if at.neg => vec![atom(gain.clone(), at.args.clone(), false), l.clone()],
                        true => vec![atom(gain.clone(), at.args.clone(), false)],
                        false => vec![l.clone()],
                    })
                    .collect();
                insert(&mut rules, h, &rule.head.args, ins, gone);
            }
            // Levels: a candidate holds when a derivation from the kept rows below it, not gone, holds.
            if lv {
                let mut held = vec![atom(named(h, "chk"), rule.head.args.clone(), false)];
                held.extend(rule.body.iter().map(|l| match l {
                    Literal::Atom(b) if !b.neg && scc.contains(b.rel.as_str()) => atom(named(&b.rel, "live"), b.args.clone(), false),
                    other => other.clone(),
                }));
                rules.push(Rule { head: Atom { rel: named(h, "held").into(), args: rule.head.args.clone(), neg: false, gated: false }, body: held });
            }
            for b in rule.body.iter().filter_map(|l| match l {
                Literal::Atom(b) | Literal::Agg { atom: b, .. } if moves(b) && !scc.contains(b.rel.as_str()) => Some(b),
                _ => None,
            }) {
                olds.insert(b.rel.to_string());
            }
            // A support's row has one derivation (`supports`): an overdeleted row is gone, never rederived.
            if exact {
                continue;
            }
            // A projection's lost row is held while a support of it is left.
            if projection {
                let mut held = vec![atom(named(h, "lost"), rule.head.args.clone(), false)];
                held.extend(rule.body.iter().cloned());
                rules.push(Rule { head: Atom { rel: named(h, "held").into(), ..rule.head.clone() }, body: held });
                continue;
            }
            let mut red = vec![atom(named(h, gone), rule.head.args.clone(), false)];
            red.extend(rule.body.iter().cloned());
            rules.push(Rule { head: Atom { rel: named(h, "redo").into(), ..rule.head.clone() }, body: red });
        }
        let exact = exact || projection;
        for rel in scc {
            let width = part.program.decls[rel.as_str()].len();
            let v = row(width);
            olds.insert(rel.clone());
            if projection {
                rules.push(Rule { head: Atom { rel: named(rel, "del").into(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "lost"), v.clone(), false), atom(named(rel, "held"), v.clone(), true)] });
                fresh.push((rel.clone(), named(rel, "lost")));
                fresh.push((rel.clone(), named(rel, "held")));
            }
            if exact {
                exact_rels.insert(rel.clone());
            }
            rules.push(Rule { head: Atom { rel: rel.clone().into(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "old"), v.clone(), false), atom(named(rel, gone), v.clone(), true)] });
            if lv {
                // Placed in the levels stratum only: the engine's driver writes `chk` and `gone` (`plan::levels_of`),
                // and `live` starts as the kept rows.
                let gated = |rel: String| Literal::Atom(Atom { rel: rel.into(), args: v.clone(), neg: true, gated: true });
                let head = |s: &str| Atom { rel: named(rel, s).into(), args: v.clone(), neg: false, gated: false };
                rules.push(Rule { head: head("chk"), body: vec![atom(named(rel, "cand"), v.clone(), false)] });
                rules.push(Rule { head: head("gone"), body: vec![atom(named(rel, "chk"), v.clone(), false), gated(named(rel, "held"))] });
                rules.push(Rule { head: head("live"), body: vec![atom(named(rel, "old"), v.clone(), false), gated(named(rel, "gone"))] });
                for s in ["cand", "chk", "held", "gone", "live"] {
                    fresh.push((rel.clone(), named(rel, s)));
                }
                gone_of.insert(rel.clone());
            }
            rules.push(Rule { head: Atom { rel: rel.clone().into(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "ins"), v.clone(), false)] });
            if !exact {
                rules.push(Rule { head: Atom { rel: rel.clone().into(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "redo"), v, false)] });
            }
            if !lv {
                fresh.push((rel.clone(), named(rel, "del")));
            }
            fresh.push((rel.clone(), named(rel, "ins")));
            if !exact {
                fresh.push((rel.clone(), named(rel, "redo")));
            }
        }
        maint.extend(scc.iter().cloned());
        changed.extend(scc.iter().cloned());
        groups.push((scc.clone(), start, rules.len(), as_given));
    }
    // One stratum's form: its relations' deltas, each as its form makes it (maintained), or by a diff of its old rows
    // (derived again, where they are kept).
    if one.is_some() {
        for rel in sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).flatten() {
            if maint.contains(rel) {
                deltas.insert(rel.clone());
            } else if old(rel) {
                deltas.insert(rel.clone());
                olds.insert(rel.clone());
            }
        }
    }
    let mut sources = Vec::new();
    // A view's delta and old rows are the view over its input's: the input's are made or bound too.
    let input_of = |rel: &str| views.get(rel).and_then(|r| r.body[0].atom()).map(|a| a.rel.to_string());
    let view_deltas: Vec<String> = deltas.iter().filter_map(|r| input_of(r)).collect();
    let view_inputs: BTreeSet<String> = view_deltas.iter().cloned().collect();
    deltas.extend(view_deltas);
    let view_olds: Vec<String> = olds.iter().filter_map(|r| input_of(r)).collect();
    olds.extend(view_olds);
    let over_input = |rel: &str, suffix: &str| -> Rule {
        let view = &views[rel];
        let a = view.body[0].atom().expect("a view's atom");
        Rule { head: Atom { rel: named(rel, suffix).into(), ..view.head.clone() }, body: vec![atom(named(&a.rel, suffix), a.args.clone(), false)] }
    };
    for rel in &deltas {
        let v = row(part.program.decls[rel.as_str()].len());
        let head = |suffix: &str| Atom { rel: named(rel, suffix).into(), args: v.clone(), neg: false, gated: false };
        if one.is_some_and(|o| !maint.contains(rel) && (o.has_delta)(rel)) {
            // A relation of the run already: an earlier stratum's form made it, or it was given.
            if !part.program.inputs.contains(&named(rel, "minus")) {
                part.program.inputs.push(named(rel, "minus"));
                part.program.inputs.push(named(rel, "plus"));
            }
        } else if views.contains_key(rel) {
            rules.push(over_input(rel, "minus"));
            rules.push(over_input(rel, "plus"));
        } else if maint.contains(rel) {
            // A maintained relation's net change, from its own overdeleted, rederived and inserted rows.
            let body = if exact_rels.contains(rel) {
                vec![atom(named(rel, "del"), v.clone(), false), atom(named(rel, "ins"), v.clone(), true)]
            } else {
                let gone = if gone_of.contains(rel) { "gone" } else { "del" };
                vec![atom(named(rel, gone), v.clone(), false), atom(named(rel, "redo"), v.clone(), true), atom(named(rel, "ins"), v.clone(), true)]
            };
            rules.push(Rule { head: head("minus"), body });
            rules.push(Rule { head: head("plus"), body: vec![atom(named(rel, "ins"), v.clone(), false), atom(named(rel, "old"), v.clone(), true)] });
        } else if let Some((plus, minus)) = given(rel) {
            // Its delta as given: no row of it is read but those.
            for (suffix, source) in [("plus", plus), ("minus", minus)] {
                sources.push((named(rel, suffix), source, None));
                part.program.inputs.push(named(rel, suffix));
            }
        } else {
            // A view's input whose delta is not given: found from its old rows, bound below.
            if view_inputs.contains(rel) {
                olds.insert(rel.clone());
            }
            rules.push(Rule { head: head("minus"), body: vec![atom(named(rel, "old"), v.clone(), false), atom(rel.clone(), v.clone(), true)] });
            rules.push(Rule { head: head("plus"), body: vec![atom(rel.clone(), v.clone(), false), atom(named(rel, "old"), v.clone(), true)] });
        }
        fresh.push((rel.clone(), named(rel, "minus")));
        fresh.push((rel.clone(), named(rel, "plus")));
    }
    // Each maintained stratum's form derived again (decision 0090 §4): its rules as given, and its deltas against
    // its old rows; what its maintained form makes (its pieces' heads, and its relations' deltas) is replaced.
    // A maintained relation's minus and plus rules, by the relation: found once, not per stratum.
    let delta_of: std::collections::HashMap<String, &str> = deltas.iter().filter(|r| maint.contains(*r)).flat_map(|r| [(named(r, "minus"), r.as_str()), (named(r, "plus"), r.as_str())]).collect();
    let mut delta_rules: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, r) in rules.iter().enumerate() {
        if let Some(&rel) = delta_of.get(r.head.rel.as_str()) {
            delta_rules.entry(rel).or_default().push(i);
        }
    }
    for (scc, start, end, given) in groups {
        let mine: Vec<&Rule> = rules[start..end].iter().chain(scc.iter().filter_map(|s| delta_rules.get(s.as_str())).flatten().map(|&i| &rules[i])).collect();
        let owned: BTreeSet<String> = mine.iter().map(|r| r.head.rel.to_string()).chain(scc.iter().cloned()).collect();
        let reads: BTreeSet<String> = mine
            .iter()
            .flat_map(|r| r.body.iter().filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(&a.rel),
                _ => None,
            }))
            .filter(|r| !owned.contains((*r).as_str()) && (r.ends_with("__minus") || r.ends_with("__plus")))
            .map(|r| r.to_string())
            .collect();
        let mut again_deltas = Vec::new();
        for rel in scc.iter().filter(|r| deltas.contains(*r)) {
            let v = row(part.program.decls[rel.as_str()].len());
            let a = |rel: String, neg: bool| atom(rel, v.clone(), neg);
            again_deltas.push(Rule { head: Atom { rel: named(rel, "minus").into(), args: v.clone(), neg: false, gated: false }, body: vec![a(named(rel, "old"), false), a(rel.clone(), true)] });
            again_deltas.push(Rule { head: Atom { rel: named(rel, "plus").into(), args: v.clone(), neg: false, gated: false }, body: vec![a(rel.clone(), false), a(named(rel, "old"), true)] });
        }
        let olds = scc.iter().map(|r| named(r, "old")).collect();
        part.program.again.push(front::Again { scc, owned, rules: given, deltas: again_deltas, reads: reads.into_iter().collect(), olds });
    }
    for rel in &olds {
        if views.contains_key(rel) {
            rules.push(over_input(rel, "old"));
            fresh.push((rel.clone(), named(rel, "old")));
            continue;
        }
        // Named as the planner reads its statistics (`sql_col`). One bound already (an earlier form's) is read.
        if !one.is_some_and(|o| (o.has_old)(rel)) {
            let (source, held) = kept.old_source(rel, &named(rel, "old")).expect("a maintained read's old rows");
            sources.push((named(rel, "old"), source, held));
        }
        fresh.push((rel.clone(), named(rel, "old")));
        part.program.inputs.push(named(rel, "old"));
    }
    for (rel, made) in fresh {
        let decl = part.program.decls[rel.as_str()].clone();
        part.program.decls.insert(made.clone().into(), decl);
        if let Some(d) = part.domains.get(&rel).cloned() {
            part.domains.insert(made.clone(), d);
        }
        if part.program.nullary.contains(&rel) {
            part.program.nullary.insert(made);
        }
    }
    if let Ok(show) = std::env::var("O125_RULES") {
        for r in rules.iter().filter(|r| r.head.rel.starts_with(show.as_str())) {
            eprintln!("O125_RULES {r}");
        }
        for r in cross.program.rules.iter().filter(|r| r.head.rel == show) {
            eprintln!("O125_RULES given {r}");
        }
    }
    part.program.rules = rules;
    together(&mut part.program);
    let views = views.into_keys().collect();
    Ok(Maintained { program: part, sources, maintained, derived_again, recursive, again, views })
}

static BY_LEVELS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The recursive strata maintained by levels so far in this process (decision 0090), not by delete-and-rederive.
pub fn strata_by_levels() -> usize {
    BY_LEVELS.load(std::sync::atomic::Ordering::Relaxed)
}

static TOGETHER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The sets of pieces run as one component so far in this process (`together`).
pub fn pieces_together() -> usize {
    TOGETHER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Decision 0101 §3: a maintained stratum's pieces (`h__del`, `h__ins`, its deltas, …) run as one component per
/// level, not one each, so they pay one component's fixed cost (its spec, open and statistics) between them.
///
/// A piece is ordered after the pieces it reads as stratified sub-steps of the one fixpoint: a negation of a piece
/// of the same component waits for a unit gate (`n__ready`, as magic's `done_` gates do) that passes once `n` and
/// what it reads there are quiet, and reads `n` whole. A survivor (`h :- h__old, !h__del`) starts the next level:
/// it is made in place from the kept rows less the deleted ones (`plan::survivors`), which are whole by then. A
/// recursive stratum, or one with a relation made natively or by a lattice, keeps its pieces apart.
///
/// Not under a device binding: the device takes no component with gates (`plan.rs`, "a component with gates or
/// requests"), so a set placed there would run on the host. Each piece there stays a stratum of seed joins, which
/// the device runs. (The device's own form of this is one submission with a dispatch per level, not gates.)
///
/// `RERAIL_PIECES_APART` runs each piece as its own component, as before (to compare).
fn together(prog: &mut front::Program) {
    if std::env::var_os("RERAIL_PIECES_APART").is_some() {
        return;
    }
    if crate::code_rule_dsl::logic::plan::placement::device() {
        crate::code_rule_dsl::logic::profile::decision("pieces together (decision 0101 §3)", || "apart: the run's binding is the device, which takes no gates".to_owned());
        return;
    }
    let derived: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut rules_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, r) in prog.rules.iter().enumerate() {
        rules_of.entry(r.head.rel.to_string()).or_default().push(i);
    }
    let reads_of = |r: &Rule| -> Vec<(String, bool)> {
        r.body.iter().filter_map(|l| match l {
            Literal::Atom(a) => Some((a.rel.to_string(), a.neg)),
            Literal::Agg { atom: a, .. } => Some((a.rel.to_string(), true)),
            _ => None,
        }).collect()
    };
    // Each group's components, by level.
    let mut sets: Vec<(usize, BTreeSet<String>)> = Vec::new();
    'group: for (g, again) in prog.again.iter().enumerate() {
        let members: BTreeSet<&String> = again.owned.iter().filter(|r| derived.contains(*r)).collect();
        if members.len() < 2 || members.iter().any(|r| prog.natives.contains_key(*r) || prog.lattice.contains_key(*r) || prog.facts.get(*r).is_some_and(|f| !f.is_empty())) {
            continue;
        }
        let inner: BTreeMap<&String, BTreeSet<String>> = members.iter().map(|r| {
            let reads = rules_of[*r].iter().flat_map(|&i| reads_of(&prog.rules[i])).map(|(x, _)| x).filter(|x| members.contains(&x)).collect();
            (*r, reads)
        }).collect();
        // A survivor: `h :- h__old, !h__del` (or `h__gone`) with the deleted rows a piece.
        let survivor = |h: &str| rules_of[h].iter().any(|&i| {
            let r = &prog.rules[i];
            matches!(r.body.as_slice(), [Literal::Atom(a), Literal::Atom(b)] if !a.neg && b.neg && a.rel.as_str() == named(h, "old")
                && (b.rel.as_str() == named(h, "del") || b.rel.as_str() == named(h, "gone")) && members.iter().any(|m| m.as_str() == b.rel.as_str()))
        });
        // Levels, reads first; a cycle (a recursive stratum) keeps the group apart.
        let mut level: BTreeMap<&String, usize> = BTreeMap::new();
        let mut on: BTreeSet<&String> = BTreeSet::new();
        fn visit<'m>(r: &'m String, inner: &'m BTreeMap<&'m String, BTreeSet<String>>, survivor: &dyn Fn(&str) -> bool, level: &mut BTreeMap<&'m String, usize>, on: &mut BTreeSet<&'m String>) -> bool {
            if level.contains_key(r) {
                return true;
            }
            if !on.insert(r) {
                return false;
            }
            let mut l = 0;
            for x in &inner[r] {
                let Some((&x, _)) = inner.get_key_value(x) else { continue };
                if !visit(x, inner, survivor, level, on) {
                    return false;
                }
                l = l.max(level[x] + usize::from(survivor(r)));
            }
            on.remove(r);
            level.insert(r, l);
            true
        }
        for r in &members {
            if !visit(r, &inner, &survivor, &mut level, &mut on) {
                continue 'group;
            }
        }
        let mut by_level: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for (r, l) in level {
            by_level.entry(l).or_default().insert(r.clone());
        }
        sets.extend(by_level.into_values().filter(|s| s.len() > 1).map(|s| (g, s)));
    }
    // Only sets that are one component each as the strata are made (none takes in a relation outside it).
    prog.together = sets.iter().map(|(_, s)| s.clone()).collect();
    let apart: BTreeSet<usize> = front::apart(prog).into_iter().collect();
    let sets: Vec<(usize, BTreeSet<String>)> = sets.into_iter().enumerate().filter(|(i, _)| !apart.contains(i)).map(|(_, s)| s).collect();
    prog.together.clear();
    for (g, set) in sets {
        // Each negated piece's depth in the set (its gates' order), and what it reads there.
        let inner: BTreeMap<&String, BTreeSet<String>> = set.iter().map(|r| {
            (r, rules_of[r].iter().flat_map(|&i| reads_of(&prog.rules[i])).map(|(x, _)| x).filter(|x| set.contains(x)).collect())
        }).collect();
        let negated: BTreeSet<String> = set.iter().flat_map(|r| rules_of[r].iter().flat_map(|&i| reads_of(&prog.rules[i]))).filter(|(x, neg)| *neg && set.contains(x)).map(|(x, _)| x).collect();
        let reach = |n: &String| -> BTreeSet<String> {
            let (mut seen, mut todo) = (BTreeSet::new(), vec![n.clone()]);
            while let Some(v) = todo.pop() {
                if seen.insert(v.clone()) {
                    todo.extend(inner[&v].iter().cloned());
                }
            }
            seen
        };
        let depth = |n: &String| -> usize {
            fn d(v: &String, inner: &BTreeMap<&String, BTreeSet<String>>) -> usize {
                inner[v].iter().map(|x| 1 + d(x, inner)).max().unwrap_or(0)
            }
            d(n, &inner)
        };
        let ready = |n: &str| named(n, "ready");
        let mut members = set.clone();
        for n in &negated {
            let quiet = reach(n);
            let deps: BTreeSet<String> = quiet.iter().flat_map(|h| rules_of[h].iter().flat_map(|&i| reads_of(&prog.rules[i]))).filter(|(x, neg)| *neg && negated.contains(x) && x != n).map(|(x, _)| ready(&x)).collect();
            let level = depth(n);
            prog.gates.push((ready(n), crate::code_rule_dsl::logic::magic::Gate { src: None, rel: n.clone(), level, quiet: quiet.into_iter().collect(), deps: deps.into_iter().collect() }));
            prog.decls.insert(ready(n).into(), vec![("k".into(), "int".into())]);
            let wild = vec![Term::Wild; prog.decls[n.as_str()].len()];
            prog.rules.push(Rule { head: Atom { rel: ready(n).into(), args: vec![Term::Int(0)], neg: false, gated: false }, body: vec![atom(n.clone(), wild, false)] });
            members.insert(ready(n));
        }
        // A negation (or an aggregate) of a piece of the set reads it once its gate passed.
        for h in &set {
            for &i in &rules_of[h] {
                let body = std::mem::take(&mut prog.rules[i].body);
                let mut out = Vec::with_capacity(body.len() + 2);
                for l in body {
                    let n = match &l {
                        Literal::Atom(a) if a.neg && set.contains(a.rel.as_str()) => Some(a.rel.to_string()),
                        Literal::Agg { atom: a, .. } if set.contains(a.rel.as_str()) => Some(a.rel.to_string()),
                        _ => None,
                    };
                    let Some(n) = n else {
                        out.push(l);
                        continue;
                    };
                    out.push(Literal::Atom(Atom { rel: ready(&n).into(), args: vec![Term::Int(0)], neg: false, gated: false }));
                    out.push(match l {
                        Literal::Atom(a) => Literal::Atom(Atom { gated: true, ..a }),
                        Literal::Agg { var, op, over, atom } => Literal::Agg { var, op, over, atom: Atom { gated: true, ..atom } },
                        l => l,
                    });
                }
                prog.rules[i].body = out;
            }
        }
        prog.again[g].owned.extend(members.iter().cloned());
        prog.together.push(members);
        TOGETHER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}
