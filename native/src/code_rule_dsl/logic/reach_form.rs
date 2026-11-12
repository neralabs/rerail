//! The reach index's form found in a program (decision 0115): the relations held as each start's closure, not as rows.
//!
//! A relation `r` of two columns is a reach when one of its rules is `r(x, z) :- r(x, y), edge(y, z)` (three distinct
//! variables, `edge` not reading `r`) and no other rule reads `r`. Its other rules become the rules of
//! `rr_reach_seed_<r>`, and `r` is made natively (`Native::Reach`) from the seeds and the edges; its one rule left,
//! `r(x, z) :- rr_reach_seed_<r>(x, y), edge(y, z)`, orders it after both. A relation `s` whose one rule is
//! `s(x, q) :- r(x, e), map(e, q)` over a reach `r` is made through it (`Native::ReachThrough`), its rule kept as the
//! order.
//!
//! `.decl r(…) reach` asks for the form: a relation so declared that does not have it is an error, so a rule edited
//! out of the shape is found, not run as rows. `.decl r(…) no_reach` keeps `r` as rows. `RERAIL_REACH_INDEX=off` finds
//! none but those declared `reach`.
use crate::code_rule_dsl::logic::front::{Atom, Error, Literal, Native, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// The prefix of a reach's seed relation.
pub const SEED: &str = "rr_reach_seed_";
/// The qualifier asking for the form.
pub const QUALIFIER: &str = "reach";

/// Each relation found in the form, rewritten. Returns the reaches and the relations through them.
pub fn find(prog: &mut Program) -> Result<(usize, usize), Error> {
    let auto = !std::env::var("RERAIL_REACH_INDEX").is_ok_and(|v| v == "off");
    let asked: BTreeSet<String> = prog.qualifiers.iter().filter(|(_, w)| w.contains(QUALIFIER)).map(|(r, _)| r.clone()).collect();
    let refused: BTreeSet<String> = prog.qualifiers.iter().filter(|(_, w)| w.contains("no_reach")).map(|(r, _)| r.clone()).collect();
    let mut reaches = 0;
    let candidates: Vec<String> = prog.decls.iter().filter(|(r, c)| c.len() == 2 && !refused.contains(*r)).map(|(r, _)| r.clone()).collect();
    for rel in &candidates {
        if !auto && !asked.contains(rel) {
            continue;
        }
        match reach_shape(prog, rel) {
            Ok((at, edge)) => {
                rewrite_reach(prog, rel, at, &edge);
                reaches += 1;
            }
            Err(why) if asked.contains(rel) && !is_through(prog, rel) => return Err(Error::Syntax(format!(".decl {rel} reach: {why}"))),
            Err(_) => {}
        }
    }
    let mut throughs = 0;
    for rel in &candidates {
        if prog.natives.contains_key(rel) || (!auto && !asked.contains(rel)) {
            continue;
        }
        match through_shape(prog, rel) {
            Ok((reach, map)) => {
                prog.natives.insert(rel.clone(), Native::ReachThrough { reach, map });
                throughs += 1;
            }
            Err(why) if asked.contains(rel) => return Err(Error::Syntax(format!(".decl {rel} reach: {why}"))),
            Err(_) => {}
        }
    }
    Ok((reaches, throughs))
}

/// Whether `rel`'s rules could be a relation through a reach (its one rule reads one), for the directive's error.
fn is_through(prog: &Program, rel: &str) -> bool {
    let rules: Vec<&Rule> = prog.rules.iter().filter(|r| r.head.rel == rel).collect();
    rules.len() == 1 && rules[0].body.len() == 2
}

fn var(t: &Term) -> Option<&str> {
    match t {
        Term::Var(v) => Some(v.as_str()),
        _ => None,
    }
}

/// The two positive atoms of a body of exactly two literals.
fn two_atoms(rule: &Rule) -> Option<(&Atom, &Atom)> {
    match rule.body.as_slice() {
        [Literal::Atom(a), Literal::Atom(b)] if !a.neg && !b.neg && !a.gated && !b.gated => Some((a, b)),
        _ => None,
    }
}

/// Why `rel` may not be held otherwise than as its rules make it: what the planner reads of it by name.
fn held_by_name(prog: &Program, rel: &str) -> Option<&'static str> {
    if prog.natives.contains_key(rel) {
        Some("made natively already")
    } else if prog.inputs.iter().any(|i| i == rel) {
        Some("an input")
    } else if prog.facts.get(rel).is_some_and(|f| !f.is_empty()) {
        Some("it has facts")
    } else if prog.lattice.contains_key(rel) {
        Some("a lattice")
    } else if prog.demand.iter().any(|d| d == rel) || prog.gates.iter().any(|(g, _)| g == rel) || prog.answers.contains_key(rel) {
        Some("demanded, a gate or an answer")
    } else {
        None
    }
}

/// The relations each relation's rules read.
fn reads(prog: &Program) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for r in &prog.rules {
        let e = out.entry(r.head.rel.as_str()).or_default();
        for l in &r.body {
            if let Some(a) = l.atom() {
                e.insert(a.rel.as_str());
            }
        }
    }
    for (rel, n) in &prog.natives {
        out.entry(rel.as_str()).or_default().extend(n.reads());
    }
    out
}

/// Whether `from` reads `to`, directly or through other relations.
fn depends(reads: &BTreeMap<&str, BTreeSet<&str>>, from: &str, to: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut todo = vec![from];
    while let Some(r) = todo.pop() {
        if r == to {
            return true;
        }
        if seen.insert(r) {
            todo.extend(reads.get(r).into_iter().flatten().copied());
        }
    }
    false
}

/// A relation `rel`'s rules read (but `rel` itself) that reads `rel` back: `rel` is in a cycle of others.
fn cycle_through(prog: &Program, rel: &str) -> Option<String> {
    let reads = reads(prog);
    let direct: BTreeSet<&str> = reads.get(rel).into_iter().flatten().copied().filter(|r| *r != rel).collect();
    direct.into_iter().find(|r| depends(&reads, r, rel)).map(str::to_owned)
}

/// The recursive rule's place and its edge relation, or why `rel` is not a reach.
fn reach_shape(prog: &Program, rel: &str) -> Result<(usize, String), String> {
    if let Some(why) = held_by_name(prog, rel) {
        return Err(why.into());
    }
    let mut recursive = None;
    let mut seeds = 0;
    for (i, r) in prog.rules.iter().enumerate().filter(|(_, r)| r.head.rel == rel) {
        if !r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == rel)) {
            seeds += 1;
            continue;
        }
        if recursive.is_some() {
            return Err("more than one rule reads it".into());
        }
        let Some((a, b)) = two_atoms(r) else { return Err("its recursive rule is not `r(x, z) :- r(x, y), edge(y, z)`".into()) };
        let (step, edge) = if a.rel == rel { (a, b) } else { (b, a) };
        let (Some(hx), Some(hz)) = (var(&r.head.args[0]), var(&r.head.args[1])) else { return Err("its recursive rule's head is not two variables".into()) };
        let shape = step.args.len() == 2
            && edge.args.len() == 2
            && edge.rel != rel
            && var(&step.args[0]) == Some(hx)
            && var(&step.args[1]).is_some_and(|y| var(&edge.args[0]) == Some(y) && y != hx && y != hz)
            && var(&edge.args[1]) == Some(hz)
            && hx != hz;
        if !shape {
            return Err("its recursive rule is not `r(x, z) :- r(x, y), edge(y, z)`".into());
        }
        recursive = Some((i, edge.rel.to_string()));
    }
    let Some((at, edge)) = recursive else { return Err("no rule reads it".into()) };
    if seeds == 0 {
        return Err("no rule seeds it".into());
    }
    // Its only cycle is its own rule: nothing its rules read (the seeds' bodies and the edges) reads it back.
    if let Some(back) = cycle_through(prog, rel) {
        return Err(format!("`{back}` reads it and it reads `{back}`"));
    }
    Ok((at, edge))
}

/// `rel`'s seeds as `rr_reach_seed_<rel>`'s rules, and `rel` made natively from them and `edge`.
fn rewrite_reach(prog: &mut Program, rel: &str, at: usize, edge: &str) {
    let seed = format!("{SEED}{rel}");
    for r in prog.rules.iter_mut().filter(|r| r.head.rel == rel) {
        r.head.rel = seed.clone().into();
    }
    let v = |n: &str| Term::Var(n.to_owned());
    let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned().into(), args, neg: false, gated: false });
    // Its one rule only orders it after the seeds and the edges: the evaluator makes its rows.
    prog.rules[at] = Rule {
        head: Atom { rel: rel.to_owned().into(), args: vec![v("x"), v("z")], neg: false, gated: false },
        body: vec![atom(&seed, vec![v("x"), v("y")]), atom(edge, vec![v("y"), v("z")])],
    };
    let cols = prog.decls[rel].clone();
    prog.decls.insert(seed.clone(), cols);
    prog.natives.insert(rel.to_owned(), Native::Reach { seed, edge: edge.to_owned() });
}

/// The reach and the map `rel` is made through, or why it is not.
fn through_shape(prog: &Program, rel: &str) -> Result<(String, String), String> {
    if let Some(why) = held_by_name(prog, rel) {
        return Err(why.into());
    }
    let rules: Vec<&Rule> = prog.rules.iter().filter(|r| r.head.rel == rel).collect();
    let [r] = rules.as_slice() else { return Err("not one rule".into()) };
    let Some((a, b)) = two_atoms(r) else { return Err("its rule is not `s(x, q) :- r(x, e), map(e, q)`".into()) };
    let is_reach = |x: &Atom| matches!(prog.natives.get(x.rel.as_str()), Some(Native::Reach { .. }));
    let (reach, map) = if is_reach(a) { (a, b) } else if is_reach(b) { (b, a) } else { return Err("its rule reads no reach".into()) };
    let (Some(hx), Some(hq)) = (var(&r.head.args[0]), var(&r.head.args[1])) else { return Err("its head is not two variables".into()) };
    let shape = map.args.len() == 2
        && map.rel != rel
        && !is_reach(map)
        && var(&reach.args[0]) == Some(hx)
        && var(&reach.args[1]).is_some_and(|e| var(&map.args[0]) == Some(e) && e != hx && e != hq)
        && var(&map.args[1]) == Some(hq)
        && hx != hq;
    if !shape {
        return Err("its rule is not `s(x, q) :- r(x, e), map(e, q)`".into());
    }
    if let Some(back) = cycle_through(prog, rel) {
        return Err(format!("`{back}` reads it and it reads `{back}`"));
    }
    Ok((reach.rel.to_string(), map.rel.to_string()))
}
