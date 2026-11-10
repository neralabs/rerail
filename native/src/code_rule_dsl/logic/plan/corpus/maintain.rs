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
use std::collections::BTreeSet;
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
    let head = Atom { rel: named(h, "ins"), args: args.to_vec(), neg: false, gated: false };
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

/// `RERAIL_RECURSIVE=levels`: recursive strata maintained by levels (decision 0090), not delete-and-rederive.
fn levels_wanted() -> bool {
    std::env::var("RERAIL_RECURSIVE").is_ok_and(|v| v == "levels")
}

fn atom(rel: String, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel, args, neg, gated: false })
}

fn row(width: usize) -> Vec<Term> {
    (0..width).map(|i| Term::Var(format!("_m{i}"))).collect()
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
    // Counting (`supports`) has no witness of a row's level: a recursive stratum is derived again.
    if super::supports::counting() && rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(&a.rel)))) {
        return Some("recursive, under counting");
    }
    for rule in rules {
        if !rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) {
            return Some("a head term");
        }
        for l in &rule.body {
            match l {
                Literal::Atom(a) if a.gated => return Some("a gated negation"),
                Literal::Atom(a) if changed.contains(&a.rel) && !scc.contains(&a.rel) && !old(&a.rel) => return Some("a changed read's old rows not kept"),
                Literal::Nearest { .. } if super::retain::TREE.iter().any(|r| changed.contains(*r)) => return Some("a nearest literal over a changed tree"),
                Literal::Atom(_) | Literal::Cmp { .. } | Literal::Nearest { .. } => {}
                // An aggregate over an earlier stratum's relation (never this one's: aggregation is not recursive).
                Literal::Agg { atom: a, .. } if a.gated => return Some("a gated negation"),
                Literal::Agg { atom: a, .. } if scc.contains(&a.rel) => return Some("an aggregate over its own stratum"),
                Literal::Agg { atom: a, .. } if a.args.iter().any(|t| matches!(t, Term::Call(..))) => return Some("an aggregate over a computed term"),
                Literal::Agg { atom: a, .. } if changed.contains(&a.rel) && !old(&a.rel) => return Some("a changed read's old rows not kept"),
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
            let v = Term::Var(format!("_h{i}"));
            let call = std::mem::replace(t, v.clone());
            rule.body.push(Literal::Cmp { l: v, op: "=", r: call });
        }
    }
    rule
}

/// `part` (the cut's program) with each changed stratum maintained where it can be, and the old rows it reads.
/// An input's delta given whole (an earlier phase's, `phases::Versioned`): its rows new and its rows gone, bound as
/// `r__plus` and `r__minus` in place of their diff of every old and new row.
pub(in crate::code_rule_dsl) type Given = std::collections::HashMap<String, (Arc<dyn Source>, Arc<dyn Source>)>;

/// `given` answers for an input whose delta it has (`Given`'s, or found by its documents), asked only for the inputs a
/// maintained stratum reads the delta of.
pub(in crate::code_rule_dsl) fn rewrite(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered, given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>) -> Result<Maintained> {
    let mut part = cut.program(cross);
    let (sccs, _) = front::strata(&cross.program).map_err(|e| Error::Plan(e.to_string()))?;
    let dirty: Vec<&BTreeSet<String>> = sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).collect();
    let why = if kept.is_none() { "nothing kept" } else { "the program has gates" };
    let Some(kept) = kept.filter(|_| cross.program.gates.is_empty()) else {
        let again = dirty.iter().map(|scc| (scc.iter().next().cloned().unwrap_or_default(), why)).collect();
        return Ok(Maintained { program: part, sources: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0, again });
    };
    let old = |rel: &str| kept.has_old(rel);
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
    for scc in dirty {
        let given: Vec<&Rule> = part.program.rules.iter().filter(|r| scc.contains(&r.head.rel)).collect();
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
            let name = scc.iter().max_by_key(|r| part.program.rules.iter().filter(|x| &x.head.rel == *r).count()).cloned().unwrap_or_default();
            again.push((format!("{name} ({} relations)", scc.len()), why));
            derived_again += 1;
            rules.extend(given.into_iter().cloned());
            changed.extend(scc.iter().cloned());
            continue;
        }
        maintained += 1;
        let exact = scc.iter().all(|r| super::supports::is_support(r));
        let is_recursive = mine.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(&a.rel))));
        if is_recursive {
            recursive += 1;
        }
        // Decision 0090: a recursive stratum's rows gone are decided in level order (`levels`) rather than
        // overdeleted, when its rules are joins, negations and comparisons and every kept row has its level.
        let lv = is_recursive && levels_wanted() && mine.iter().all(|r| r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. })))
            && scc.iter().all(|r| kept.kept(r).is_some_and(|(h, _)| h.has_levels()));
        if is_recursive && levels_wanted() && std::env::var_os("RERAIL_LEVELS_WHY").is_some() {
            let not_join = mine.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).count();
            let no_levels: Vec<(&String, (usize, usize, bool))> = scc.iter().filter_map(|r| kept.kept(r).filter(|(h, _)| !h.has_levels()).map(|(h, _)| (r, h.levels_shape()))).take(4).collect();
            let unkept: Vec<&String> = scc.iter().filter(|r| kept.kept(r).is_none()).take(4).collect();
            eprintln!("RERAIL_LEVELS_WHY {:?}: levels {lv}; rules not joins {not_join}; without levels {no_levels:?}; not kept {unkept:?}", scc.iter().next());
        }
        let gone = if lv { "gone" } else { "del" };
        // A read that can change: an atom of this stratum, or a changed relation (positive or negated).
        let moves = |a: &Atom| scc.contains(&a.rel) || changed.contains(&a.rel);
        let old_of = |a: &Atom| if moves(a) { named(&a.rel, "old") } else { a.rel.clone() };
        // A literal over old rows: an atom, or an aggregate's atom, that moves reads `r__old`.
        let old_lit = |l: &Literal| match l {
            Literal::Atom(b) => atom(old_of(b), b.args.clone(), b.neg),
            Literal::Agg { var, op, over, atom: b } => {
                Literal::Agg { var: var.clone(), op: op.clone(), over: over.clone(), atom: Atom { rel: old_of(b), ..b.clone() } }
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
                    let cols = &part.program.decls[&at.rel];
                    let mut decl: Vec<(String, String)> = g
                        .iter()
                        .map(|v| {
                            let c = at.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).expect("a group variable is the atom's");
                            (v.clone(), cols[c].1.clone())
                        })
                        .collect();
                    let mut key: Vec<Term> = g.iter().cloned().map(Term::Var).collect();
                    if decl.is_empty() {
                        decl.push(("rr_unit".to_owned(), "int".to_owned()));
                        key.push(Term::Int(1));
                        part.program.nullary.insert(grp.clone());
                    }
                    part.program.decls.insert(grp.clone(), decl);
                    for d in ["minus", "plus"] {
                        rules.push(Rule { head: Atom { rel: grp.clone(), args: key.clone(), neg: false, gated: false }, body: vec![atom(named(&at.rel, d), at.args.clone(), false)] });
                    }
                    deltas.insert(at.rel.clone());
                    let mut over_old = vec![atom(grp.clone(), key.clone(), false)];
                    over_old.extend(rule.body.iter().map(old_lit));
                    rules.push(Rule { head: Atom { rel: named(h, "del"), args: rule.head.args.clone(), neg: false, gated: false }, body: over_old });
                    let mut over_new = vec![atom(grp, key, false)];
                    over_new.extend(rule.body.iter().cloned());
                    insert(&mut rules, h, &rule.head.args, over_new, "del");
                    continue;
                }
                let Literal::Atom(at) = lit else { continue };
                if !moves(at) {
                    continue;
                }
                let (lose, gain) = match (at.neg, scc.contains(&at.rel)) {
                    (false, true) => (named(&at.rel, "del"), named(&at.rel, "ins")),
                    (false, false) => (named(&at.rel, "minus"), named(&at.rel, "plus")),
                    (true, _) => (named(&at.rel, "plus"), named(&at.rel, "minus")),
                };
                if !scc.contains(&at.rel) {
                    deltas.insert(at.rel.clone());
                }
                // Under levels a premise of the stratum is lost when it is gone, and the head is a candidate.
                let lose = if lv && !at.neg && scc.contains(&at.rel) { named(&at.rel, "gone") } else { lose };
                let over: Vec<Literal> = rule
                    .body
                    .iter()
                    .enumerate()
                    .map(|(j, l)| match l {
                        Literal::Atom(b) if j == k => atom(lose.clone(), b.args.clone(), false),
                        other => old_lit(other),
                    })
                    .collect();
                rules.push(Rule { head: Atom { rel: named(h, if lv { "cand" } else { "del" }), args: rule.head.args.clone(), neg: false, gated: false }, body: over });
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
                    Literal::Atom(b) if !b.neg && scc.contains(&b.rel) => atom(named(&b.rel, "live"), b.args.clone(), false),
                    other => other.clone(),
                }));
                rules.push(Rule { head: Atom { rel: named(h, "held"), args: rule.head.args.clone(), neg: false, gated: false }, body: held });
            }
            for b in rule.body.iter().filter_map(|l| match l {
                Literal::Atom(b) | Literal::Agg { atom: b, .. } if moves(b) && !scc.contains(&b.rel) => Some(b),
                _ => None,
            }) {
                olds.insert(b.rel.clone());
            }
            // A support's row has one derivation (`supports`): an overdeleted row is gone, never rederived.
            if exact {
                continue;
            }
            let mut red = vec![atom(named(h, gone), rule.head.args.clone(), false)];
            red.extend(rule.body.iter().cloned());
            rules.push(Rule { head: Atom { rel: named(h, "redo"), ..rule.head.clone() }, body: red });
        }
        for rel in scc {
            let width = part.program.decls[rel].len();
            let v = row(width);
            olds.insert(rel.clone());
            rules.push(Rule { head: Atom { rel: rel.clone(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "old"), v.clone(), false), atom(named(rel, gone), v.clone(), true)] });
            if lv {
                // Placed in the levels stratum only: the engine's driver writes `chk` and `gone` (`plan::levels_of`),
                // and `live` starts as the kept rows.
                let gated = |rel: String| Literal::Atom(Atom { rel, args: v.clone(), neg: true, gated: true });
                let head = |s: &str| Atom { rel: named(rel, s), args: v.clone(), neg: false, gated: false };
                rules.push(Rule { head: head("chk"), body: vec![atom(named(rel, "cand"), v.clone(), false)] });
                rules.push(Rule { head: head("gone"), body: vec![atom(named(rel, "chk"), v.clone(), false), gated(named(rel, "held"))] });
                rules.push(Rule { head: head("live"), body: vec![atom(named(rel, "old"), v.clone(), false), gated(named(rel, "gone"))] });
                for s in ["cand", "chk", "held", "gone", "live"] {
                    fresh.push((rel.clone(), named(rel, s)));
                }
                gone_of.insert(rel.clone());
            }
            rules.push(Rule { head: Atom { rel: rel.clone(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "ins"), v.clone(), false)] });
            if !exact {
                rules.push(Rule { head: Atom { rel: rel.clone(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "redo"), v, false)] });
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
    }
    let mut sources = Vec::new();
    for rel in &deltas {
        let v = row(part.program.decls[rel].len());
        let head = |suffix: &str| Atom { rel: named(rel, suffix), args: v.clone(), neg: false, gated: false };
        if maint.contains(rel) {
            // A maintained relation's net change, from its own overdeleted, rederived and inserted rows.
            let body = if super::supports::is_support(rel) {
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
            rules.push(Rule { head: head("minus"), body: vec![atom(named(rel, "old"), v.clone(), false), atom(rel.clone(), v.clone(), true)] });
            rules.push(Rule { head: head("plus"), body: vec![atom(rel.clone(), v.clone(), false), atom(named(rel, "old"), v.clone(), true)] });
        }
        fresh.push((rel.clone(), named(rel, "minus")));
        fresh.push((rel.clone(), named(rel, "plus")));
    }
    for rel in &olds {
        // Named as the planner reads its statistics (`sql_col`).
        let (source, held) = kept.old_source(rel, &named(rel, "old")).expect("a maintained read's old rows");
        sources.push((named(rel, "old"), source, held));
        fresh.push((rel.clone(), named(rel, "old")));
        part.program.inputs.push(named(rel, "old"));
    }
    for (rel, made) in fresh {
        let decl = part.program.decls[&rel].clone();
        part.program.decls.insert(made.clone(), decl);
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
    Ok(Maintained { program: part, sources, maintained, derived_again, recursive, again })
}
