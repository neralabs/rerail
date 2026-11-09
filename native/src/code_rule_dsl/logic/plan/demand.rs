//! Demand for negated relations (decision 0048): a relation computed for every node that rules only negate is
//! computed for the keys those rules reach, not for every node.
//!
//! An imperfect conversion leaves relations like
//!
//! ```text
//! blocked($low, $high) :- nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- <body binding $low, $high>, !blocked($low, $high).
//! ```
//!
//! where `blocked` holds a row for every node (1.8M rows on 1000 files) and the reader asks about a few. Each reader
//! is split into its body less the negations it demands, and the negations:
//!
//! ```text
//! pre_<i>(vars) :- <the reader's body without its negations of demanded relations>.
//! demand_blocked($low, $high) :- pre_<i>(vars).
//! blocked($low, $high) :- demand_blocked($low, $high), nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- pre_<i>(vars), !blocked($low, $high).
//! ```
//!
//! Still a keyed anti-join, run set at a time (the spike: a per-row probe re-runs the body per outer row, S203/S064;
//! a keyed anti-join beats a subplan rescan, S254/S272). The guard is structural, not statistics (history index
//! §4): `pre_<i>` is the reader's own work less some anti-joins, planned freely in every chunk, so the demand never
//! exceeds what the reader binds. No plan is frozen into the split (a plan taken from one chunk ordered the body
//! badly in others: 320M rows), and a relation negated twice in one rule needs no copy, since every demand comes from
//! the one `pre_<i>`.
//!
//! A relation is demanded when it is derived and not kept, not recursive, read, and read only by atoms (negated or
//! positive) with variable or constant arguments, and when it is large:
//!
//! - every rule of it starts from every node (a `nearest_<set>`, `inside_<set>` or kind-free `node` atom on its key),
//!   or
//! - its scans (rules where a base atom binds its key) reach at least 1/32 of the chunk's nodes together, by the
//!   measured counts of the base relations and of the program's constants, never by estimates of derived relations.
//!   Its other rules take the demand too. The rows not built must outweigh the strata the split adds, opened in
//!   every chunk.
//!
//! The program overrides the size test (decision 0099): `.decl r(…) magic` demands r whenever it can be, and
//! `.decl r(…) no_magic` never.
//!
//! A positive read is cut like a negation, with a guard of its own. A negation only filters, so the reader's body less
//! it is no larger than the reader's work. A positive read can be what keeps the body small, so it is cut only where
//!
//! - the kept anchors bind all its variables, directly or through lookups of bounded fan-out (`reached`), and
//! - every kept positive atom is so reached, or the reader is itself demanded (its `pre_<i>` then starts from that
//!   demand).
//!
//! An anchor is a derived relation, or a base atom whose measured rows are fewer than 1/32 of the nodes. A text test is
//! not one (`*[a-z]*` matched 7.2M of 7.2M names), nor a common kind (`node(_, $n, 1025, _, _)`: 6M identifiers), nor
//! an axis (`inside_ks5` from the rare kind 1132 reached 18.4M nodes). Where only another demanded read anchors a read,
//! that one is kept in the body instead. For example
//!
//! ```text
//! m131($n) :- child($n, $c, _), node(_, $c, 1025, _, _), text($c, _).   // 6.0M rows on the full corpus
//! m132($n) :- m126($n), m131($n).                                          // m126: 242k rows
//! ```
//!
//! becomes `pre_i($n) :- m126($n)`, `demand_m131($n) :- pre_i($n)`, `m131` over `demand_m131`, and
//! `m132($n) :- pre_i($n), m131($n)`. Corpus runs only, so the golden plans are unchanged.

use super::super::front::{Atom, Literal, Program, Rule, Term, Value};
use std::collections::{BTreeMap, BTreeSet};

thread_local! {
    /// Set while a session evaluates its cross-file part (decision 0077): the rows it keeps are each relation's whole
    /// rows, which delete-and-rederive maintains, so no relation is computed only for the keys its readers reach. A
    /// relation kept for some keys would miss a key demanded later whose derivation no delta touches (spike O125).
    static WHOLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While the guard lives, `split_negations` splits nothing (`WHOLE`).
pub(in crate::code_rule_dsl) fn whole_relations() -> impl Drop {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            WHOLE.with(|w| w.set(self.0));
        }
    }
    Reset(WHOLE.with(|w| w.replace(true)))
}

/// `prog` with each demandable relation computed for the keys its readers reach (the module's split), and what was
/// demanded; `None` where nothing is, or where the split would not stratify or type.
pub(super) fn split_negations(prog: &Program, model: &super::Model) -> Option<(Program, Vec<String>)> {
    if WHOLE.with(|w| w.get()) {
        return None;
    }
    let kept = |rel: &str| {
        prog.outputs.iter().any(|o| o == rel)
            || prog.inputs.iter().any(|i| i == rel)
            || prog.lattice.contains_key(rel)
            || prog.natives.contains_key(rel)
            || prog.facts.get(rel).is_some_and(|f| !f.is_empty())
            || prog.demand.iter().any(|d| d == rel)
            || prog.gates.iter().any(|(g, _)| g == rel)
            // `.decl r(…) no_magic` (decision 0099): the program keeps it whole.
            || prog.qualified(rel, "no_magic")
            // A relation memoised per import component (decision 0076, its copy from `<rel>__cross`): its stored rows
            // must be all of a document's, not those a reader outside the closure asked for.
            || prog.rules.iter().any(|r| r.head.rel == rel && r.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.ends_with("__cross"))))
    };
    let mut by_rel: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for rule in &prog.rules {
        by_rel.entry(rule.head.rel.as_str()).or_default().push(rule);
    }
    // Starts from every node: a `nearest_<set>`, `inside_<set>` or kind-free `node` atom on its key.
    let per_node = |rule: &Rule| -> bool {
        let Some(Term::Var(key)) = rule.head.args.first() else { return false };
        let on_key = |a: &Atom, c: usize| a.args.get(c) == Some(&Term::Var(key.clone()));
        rule.body.iter().any(|l| match l {
            Literal::Atom(a) if !a.neg => {
                ((a.rel.starts_with("nearest_") || a.rel.starts_with(super::super::front::AXIS)) && on_key(a, 0))
                    || (a.rel == "node" && on_key(a, 1) && a.args.get(2) == Some(&Term::Wild))
            }
            Literal::Nearest { n, .. } => n == key,
            _ => false,
        })
    };
    // A scan of the corpus: per head variable, the fewest rows of a base atom that binds it (measured counts of the
    // base relations and of the program's constants, not estimates of derived relations), and the most of those over
    // the head's variables; `None` when no base atom binds one. Not the key's alone: in
    // `restores(s, x, a) :- binding(s, x, site, _), site != a, earlier(site, a)` a derived atom binds the key `s`,
    // and `earlier`'s `docof(a, d)` makes a row for every node after each site (1.6 M rows on the Python pack's
    // fixtures, for a reader that asks about 483).
    let scan = |rule: &Rule| -> Option<f64> {
        rule.head
            .args
            .iter()
            .filter(|t| matches!(t, Term::Var(_)))
            .filter_map(|v| {
                rule.body
                    .iter()
                    .filter_map(|l| match l {
                        Literal::Atom(a) if !a.neg && base(prog, &a.rel) && a.args.contains(v) => Some(base_rows(prog, model, a)),
                        _ => None,
                    })
                    .reduce(f64::min)
            })
            .reduce(f64::max)
    };
    // A scan is worth demanding when it reaches at least this many rows: the rows not built must outweigh the strata
    // the split adds (`pre_<i>` and `demand_<r>`, opened in every chunk).
    let large = model.stats.nodes / 32.0;
    // Its scan rules reach `large` together; its other rules (a join of derived relations, `m132(n) :- pre(n), m131(n)`)
    // take the demand too, which only narrows them.
    let demandable = |rules: &[&Rule]| -> bool {
        let rows = rules.iter().filter_map(|r| scan(r)).sum::<f64>();
        rules.iter().all(|r| per_node(r)) || (rows > 0.0 && rows >= large)
    };
    let recursive = |rel: &str, rules: &[&Rule]| rules.iter().any(|r| r.body.iter().any(|l| matches!(l, Literal::Atom(a) | Literal::Agg { atom: a, .. } if a.rel == rel)));
    let mut candidates: BTreeSet<&str> = by_rel
        .iter()
        .filter(|(rel, rules)| !kept(rel) && !recursive(rel, rules) && rules.iter().all(|r| !reads_demand(r)) && (demandable(rules) || prog.qualified(rel, "magic")))
        .map(|(rel, _)| *rel)
        .collect();
    // Read only by atoms (negated or positive) whose arguments are variables or constants.
    for rule in &prog.rules {
        for l in &rule.body {
            match l {
                Literal::Atom(a) if a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) => {}
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    candidates.remove(a.rel.as_str());
                }
                _ => {}
            }
        }
    }
    // Read somewhere: a relation nothing reads stays as it is.
    let read: BTreeSet<&str> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
    candidates.retain(|r| read.contains(r));
    if candidates.is_empty() {
        return None;
    }
    // A candidate whose split fails (a variable of no known type, or a cycle through a negation) is left as it was,
    // and the rest are split again: one rule must not keep every other from its demand (in a combined program, one
    // component closing a cycle left all 100 rules without one).
    loop {
        match split_with(prog, model, &candidates) {
            Ok(split) => return Some(split),
            Err(failed) => {
                let before = candidates.len();
                candidates.retain(|c| !failed.contains(*c));
                if candidates.is_empty() || candidates.len() == before {
                    return None;
                }
            }
        }
    }
}

/// Whether the positive atoms `cut` leaves fall apart: two of them share no variable through the others (or
/// through an equality).
fn disconnected(rule: &Rule, cut: impl Fn(usize) -> bool) -> bool {
    // A document variable does not connect: a document has a row for each of its nodes, so atoms joined only by
    // their documents (`docof($s, $e9), rr_cx_reach($d9, $e9)` and a row of `$d9`'s) are a product per pair of
    // documents. 4,719 `anc` rows times the annotations of every document they reach made 24M rows (O136).
    let documents: BTreeSet<String> = rule
        .body
        .iter()
        .filter_map(Literal::atom)
        .filter(|a| !a.neg)
        .flat_map(|a| match a.rel.as_str() {
            "docof" => a.args.get(1).into_iter().collect::<Vec<_>>(),
            r if r.starts_with("rr_cx_") => a.args.iter().collect(),
            _ => Vec::new(),
        })
        .filter_map(|t| match t {
            Term::Var(v) => Some(v.clone()),
            _ => None,
        })
        .collect();
    let mut groups: Vec<BTreeSet<String>> = Vec::new();
    for (j, l) in rule.body.iter().enumerate() {
        let joins = match l {
            Literal::Atom(a) => !a.neg && !cut(j),
            Literal::Cmp { op: "=", .. } => true,
            _ => false,
        };
        let vars: BTreeSet<String> = l.vars().into_iter().filter(|v| !documents.contains(v)).collect();
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

/// `prog` split for `candidates`, or the candidates whose split failed.
fn split_with(prog: &Program, model: &super::Model, candidates: &BTreeSet<&str>) -> Result<(Program, Vec<String>), BTreeSet<String>> {
    // Each reader: its reads of candidates. A negation's variables the rest of its body binds (a safe rule's do); a
    // positive read's are checked below.
    let negated = |rule: &Rule, i: usize| matches!(&rule.body[i], Literal::Atom(a) if candidates.contains(a.rel.as_str()));
    let readers: Vec<usize> = (0..prog.rules.len()).filter(|&ri| (0..prog.rules[ri].body.len()).any(|i| negated(&prog.rules[ri], i))).collect();
    // A positive read is cut only where one kept anchor binds all its variables (the module's guard). Every reader is
    // checked before any is split, so one round drops every candidate a guard refuses.
    let mut refused = BTreeSet::new();
    for &ri in &readers {
        let rule = &prog.rules[ri];
        for (i, l) in rule.body.iter().enumerate() {
            let Literal::Atom(a) = l else { continue };
            if a.neg || !negated(rule, i) {
                continue;
            }
            let vars = l.vars();
            let anchors = |demanded: bool| {
                rule.body.iter().enumerate().find_map(|(j, k)| match k {
                    Literal::Atom(b) if j != i && !b.neg && negated(rule, j) == demanded && anchor(prog, model, b) && vars.is_subset(&k.vars()) => Some(b),
                    _ => None,
                })
            };
            // Bound by the kept anchors through bounded lookups; or anchored only by another demanded read, which then
            // stays in the body instead (its relation is built whole).
            let own = candidates.contains(rule.head.rel.as_str());
            let (reach, small) = reached(prog, model, rule, own, |j| negated(rule, j));
            if anchors(false).is_none() && !vars.is_subset(&reach) {
                match anchors(true) {
                    Some(b) => refused.insert(b.rel.clone()),
                    None => refused.insert(a.rel.clone()),
                };
                continue;
            }
            // The body less the read must be small too: every kept positive atom reached from the anchors by a bounded
            // lookup (`reached`; an `inside_<set>` read from its ancestor, a rare kind, still reaches 18M nodes),
            // unless the reader is itself demanded, when its `pre_<i>` starts from that demand.
            if !own && !small {
                refused.insert(a.rel.clone());
            }
            // The body less the demanded reads must be one join: kept atoms that only a demanded read connects
            // (`p13_er_body(f, n)` and `p13_unparen(e, m)`, joined by the read alone) would make `pre_<i>` their
            // cross product.
            if !own && disconnected(rule, |j| negated(rule, j)) {
                refused.insert(a.rel.clone());
            }
        }
    }
    if !refused.is_empty() {
        super::super::profile::decision("demand (decision 0048)", || format!("not demanded: {} (read positively where no anchor binds the read, or the rest of the body is not small)", refused.iter().cloned().collect::<Vec<_>>().join(", ")));
        return Err(refused);
    }
    let mut out = prog.clone();
    let mut rules: Vec<Rule> = Vec::new();
    let mut demand_rules: Vec<Rule> = Vec::new();
    for (ri, rule) in prog.rules.iter().enumerate() {
        if !readers.contains(&ri) {
            rules.push(rule.clone());
            continue;
        }
        let (mut cut, mut kept): (Vec<&Literal>, Vec<&Literal>) = (Vec::new(), Vec::new());
        for (i, l) in rule.body.iter().enumerate() {
            if negated(rule, i) { cut.push(l) } else { kept.push(l) }
        }
        // A reader that is itself a demanded relation's rule: its `pre_<i>` reads that relation's demand too.
        let own = candidates.contains(rule.head.rel.as_str()).then(|| Literal::Atom(Atom { rel: format!("demand_{}", rule.head.rel), args: rule.head.args.clone(), neg: false, gated: false }));
        if let Some(own) = &own {
            kept.insert(0, own);
        }
        // A ground reader needs its keys unconditionally. Leaving the reader unchanged but guarding its
        // producer without these seeds makes a true Boolean false. No context relation is needed here.
        if kept.is_empty() {
            for literal in &cut {
                let read = literal.atom().expect("a demanded read");
                let values = read.args.iter().map(|term| match term {
                    Term::Int(value) => Some(Value::Int(*value)),
                    Term::Str(value) => Some(Value::Str(value.clone())),
                    _ => None,
                }).collect::<Option<Vec<_>>>();
                let Some(values) = values else { return Err(cut.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).collect()) };
                let rows = out.facts.entry(format!("demand_{}", read.rel)).or_default();
                if !rows.contains(&values) { rows.push(values); }
            }
            rules.push(rule.clone());
            continue;
        }
        let mut bound = BTreeSet::new();
        for l in &kept {
            bound.extend(l.vars());
        }
        // The head's variables, those inside a call (`@cat(p, s)`) too.
        let mut later = BTreeSet::new();
        rule.head.args.iter().for_each(|t| t.vars(&mut later));
        for l in &cut {
            later.extend(l.vars());
        }
        let vars: Vec<String> = bound.intersection(&later).cloned().collect();
        let Some(types) = vars.iter().map(|v| var_type(prog, &kept, v)).collect::<Option<Vec<String>>>() else {
            // A variable of no known type (bound by a comparison alone): nothing is split.
            super::super::profile::decision("demand (decision 0048)", || format!("not demanded: a variable of a rule for {} has no known type", rule.head.rel));
            return Err(cut.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).collect());
        };
        let pre = format!("pre_{ri}");
        let mut cols: Vec<(String, String)> = vars.iter().zip(&types).map(|(v, t)| (v.clone(), t.clone())).collect();
        let mut args: Vec<Term> = vars.iter().map(|v| Term::Var(v.clone())).collect();
        // A split sharing no variable with its reads is a flag, encoded as every nullary relation is: one unit column
        // (`rr::lower::lower_boolean_relations`).
        if vars.is_empty() {
            out.nullary.insert(pre.clone());
            cols.push(("rr_unit".to_owned(), "int".to_owned()));
            args.push(Term::Int(1));
        }
        out.decls.insert(pre.clone(), cols);
        let pre_atom = Atom { rel: pre, args, neg: false, gated: false };
        rules.push(Rule { head: pre_atom.clone(), body: kept.into_iter().cloned().collect() });
        let mut body = vec![Literal::Atom(pre_atom.clone())];
        for l in cut {
            let Literal::Atom(read) = l else { unreachable!("a read atom") };
            demand_rules.push(Rule {
                head: Atom { rel: format!("demand_{}", read.rel), args: read.args.clone(), neg: false, gated: false },
                body: vec![Literal::Atom(pre_atom.clone())],
            });
            body.push(l.clone());
        }
        rules.push(Rule { head: rule.head.clone(), body });
    }
    let demanded: Vec<String> = candidates.iter().map(|r| (*r).to_owned()).collect();
    for rel in &demanded {
        let name = format!("demand_{rel}");
        out.decls.insert(name.clone(), prog.decls[rel].clone());
        for rule in rules.iter_mut().filter(|r| &r.head.rel == rel) {
            rule.body.insert(0, Literal::Atom(Atom { rel: name.clone(), args: rule.head.args.clone(), neg: false, gated: false }));
        }
    }
    rules.extend(demand_rules);
    out.rules = rules;
    // A split that closes a cycle through a negation (a reader's body reading what the negated relation depends on)
    // leaves the program as it was.
    match super::super::front::strata(&out) {
        Ok(_) => Ok((out, demanded)),
        Err(e) => {
            // The candidates in the failing component (a `demand_<c>` there names `c`).
            let failed: BTreeSet<String> = match &e {
                super::super::front::Error::NotStratifiable { component, .. } | super::super::front::Error::AggregationInRecursion { component, .. } => component
                    .iter()
                    .map(|r| r.strip_prefix("demand_").unwrap_or(r))
                    .filter(|r| candidates.contains(r))
                    .map(str::to_owned)
                    .collect(),
                _ => BTreeSet::new(),
            };
            super::super::profile::decision("demand (decision 0048)", || format!("not demanded: {} (their split closes a cycle through a negation or an aggregate)", failed.iter().cloned().collect::<Vec<_>>().join(", ")));
            Err(failed)
        }
    }
}

/// Whether the body of `rule` less its demanded reads (`cut(j)`) is small: every kept positive literal is reached
/// from the kept anchors (and, where the reader is itself demanded, from its head) through lookups of bounded fan-out.
/// A base relation is read by a node it is keyed on: `node` by its id; `field` and `child` by the parent or the
/// child (a node's fields and children, its one parent); `doc` by its id or path; an axis or any other base
/// relation by its first column (a node's ancestors, its one nearest, its text). Structural, not statistics: no
/// estimate of a derived relation is read, and an axis read from its ancestor end (every descendant) is not a
/// lookup. Decision 0048's extension, since axes are made per looked-up node (decision 0057).
/// Also the variables so bound.
fn reached(prog: &Program, model: &super::Model, rule: &Rule, own: bool, cut: impl Fn(usize) -> bool) -> (BTreeSet<String>, bool) {
    let mut bound: BTreeSet<String> = BTreeSet::new();
    if own {
        rule.head.args.iter().for_each(|t| t.vars(&mut bound));
    }
    let lookup = |a: &Atom, bound: &BTreeSet<String>| {
        let on = |c: usize| match a.args.get(c) {
            Some(Term::Var(v)) => bound.contains(v),
            Some(Term::Wild) | None => false,
            Some(_) => true,
        };
        match a.rel.as_str() {
            "node" => on(1),
            "field" => on(0) || on(2),
            "child" => on(0) || on(1),
            "doc" => on(0) || on(1),
            _ => on(0),
        }
    };
    let mut todo: Vec<usize> = (0..rule.body.len())
        .filter(|&j| match &rule.body[j] {
            Literal::Atom(b) => !b.neg && !cut(j),
            Literal::Nearest { .. } => true,
            _ => false,
        })
        .collect();
    loop {
        let before = todo.len();
        todo.retain(|&j| {
            let hit = match &rule.body[j] {
                Literal::Atom(b) => anchor(prog, model, b) || lookup(b, &bound),
                Literal::Nearest { n, .. } => bound.contains(n),
                _ => true,
            };
            if hit {
                bound.extend(rule.body[j].vars());
            }
            !hit
        });
        if todo.is_empty() || todo.len() == before {
            return (bound, todo.is_empty());
        }
    }
}

/// A base relation of the corpus: an input, a `nearest_<set>` or an `inside_<set>`.
fn base(prog: &Program, rel: &str) -> bool {
    prog.inputs.iter().any(|i| i == rel) || rel.starts_with("nearest_") || rel.starts_with(super::super::front::AXIS)
}

/// The rows a base atom reaches: its relation's, or, with constants, the fewest postings of one of them.
fn base_rows(prog: &Program, model: &super::Model, a: &Atom) -> f64 {
    let all = if a.rel.starts_with("nearest_") || a.rel.starts_with(super::super::front::AXIS) { model.stats.nodes } else { model.card(&a.rel) };
    let names = prog.decls.get(&a.rel);
    a.args
        .iter()
        .enumerate()
        .filter_map(|(c, t)| {
            let v = match t {
                Term::Int(i) => super::Constant::Int(*i),
                _ => return None,
            };
            let name = &names?.get(c)?.0;
            model.stats.const_freq(&a.rel, name, &v).map(|n| n as f64)
        })
        .fold(all, f64::min)
}

/// An atom that restricts the rows a body reaches: a derived relation (not a text test, whose selectivity is unknown),
/// or a base atom whose measured rows are fewer than a scan's (`node(_, $n, 1223, _, _)`, not the 6M identifiers of
/// `node(_, $n, 1025, _, _)`).
fn anchor(prog: &Program, model: &super::Model, a: &Atom) -> bool {
    if a.rel.starts_with("strtest_") || a.rel.starts_with("demand_") {
        return false;
    }
    !base(prog, &a.rel) || base_rows(prog, model, a) < model.stats.nodes / 32.0
}

/// Whether `rule` already reads a demand (`demand_<r>`, made before the run).
fn reads_demand(rule: &Rule) -> bool {
    rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.starts_with("demand_")))
}

/// A variable's type: its column's in the first positive atom of `body` that names it.
fn var_type(prog: &Program, body: &[&Literal], var: &str) -> Option<String> {
    body.iter().find_map(|l| match l {
        Literal::Atom(a) if !a.neg => {
            let c = a.args.iter().position(|t| matches!(t, Term::Var(v) if v == var))?;
            prog.decls.get(&a.rel)?.get(c).map(|(_, t)| t.clone())
        }
        Literal::Nearest { n, a, .. } if n == var || a == var => Some("node".to_owned()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    /// A split whose kept atoms only the demanded read joins is refused: its `pre_<i>` would be their cross product.
    #[test]
    fn kept_atoms_that_only_the_cut_joins_are_disconnected() {
        let p = super::super::super::front::parse(
            r#"
            .decl body(f: node, n: node)
            .decl unp(e: node, m: node)
            .decl d(n: node, e: node)
            .decl out(f: node, e: node)
            .input body, unp, d
            .output out
            out($f, $e) :- body($f, $n), unp($e, $m), d($n, $e).
            out($f, $e) :- body($f, $n), unp($e, $n), d($n, $e).
            "#,
        )
        .unwrap();
        for (r, apart) in p.rules.iter().zip([true, false]) {
            let cut = |j: usize| matches!(&r.body[j], super::Literal::Atom(a) if a.rel == "d");
            assert_eq!(super::disconnected(r, cut), apart, "{r}");
        }
    }

    /// Atoms joined only through their documents are apart (O136): cutting `uinst` would leave every annotation of
    /// every reached document against every `anc` row.
    #[test]
    fn atoms_joined_only_by_their_documents_are_disconnected() {
        let p = super::super::super::front::parse(
            r#"
            .decl ann(f: node, x: number, t: node)
            .decl uinst(t: node, c: node)
            .decl anc(s: node, c: node)
            .decl docof(n: node, d: number)
            .decl rr_cx_reach(d: number, e: number)
            .decl bv(f: node, x: number, s: node)
            .input ann, uinst, anc, docof, rr_cx_reach
            .output bv
            bv($f, $x, $s) :- ann($f, $x, $t), uinst($t, $c), anc($s, $c), docof($f, $d9), docof($s, $e9), rr_cx_reach($d9, $e9).
            "#,
        )
        .unwrap();
        let r = &p.rules[0];
        let at = |rel: &'static str| move |j: usize| matches!(&r.body[j], super::Literal::Atom(a) if a.rel == rel);
        assert!(super::disconnected(r, at("uinst")), "cutting uinst leaves a product by documents");
        assert!(!super::disconnected(r, at("rr_cx_reach")), "the rest still joins by t and c");
    }
}
