//! Requests answered inside the fixpoint (decision 0065 §2a).
//!
//! A program's `answers` name request relations (`rr_dir_req`, `load`, …) and what an answer adds rows to
//! (`dir_entry`, `loaded`, a loaded document's per-file rows). `front::strata` counts each produced relation as
//! reading its request, so a request that depends on its own answers is one recursive component with them, and the
//! native fixpoint pauses for the runner to answer (`setfix`'s `request` and `produced`).
//!
//! A negation inside such a component is not stratified: its relation grows as answers come in. [`gate`] makes it
//! wait, per key: `!N(t)` becomes `done_N_ad(t), !N(t)`, where `ask_N_ad(t)` holds the keys the rule asks about
//! (derived from the literals before the negation) and the engine copies a key into `done_N_ad` only in a wave after
//! which nothing N reads changed and no request is unanswered (the gate demand uses, `magic::gate_negations`). This
//! is sound when `N(t)` reads only answers that `t` determines: an answered request is whole, and later answers add
//! new keys rather than rows under an old one. Import resolution is of that kind.

use super::front::{Atom, Error, Literal, Program, Rule, Term, tarjan};
use super::magic::Gate;
use std::collections::{BTreeMap, BTreeSet};

fn atom(rel: String, args: Vec<Term>) -> Atom {
    Atom { rel, args, neg: false, gated: false }
}

/// Every produced relation, with its request.
pub(in crate::code_rule_dsl) fn produced(prog: &Program) -> BTreeMap<&str, &str> {
    prog.answers.iter().flat_map(|(req, ps)| ps.iter().map(move |p| (p.as_str(), req.as_str()))).collect()
}

/// Gates the negations inside each component that holds a request (module docs). A program without answers is
/// unchanged.
pub(in crate::code_rule_dsl) fn gate(prog: &mut Program) -> Result<(), Error> {
    if prog.answers.is_empty() {
        return Ok(());
    }
    let produced: BTreeMap<String, String> = produced(prog).into_iter().map(|(p, r)| (p.to_owned(), r.to_owned())).collect();
    let mut nodes: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).collect();
    nodes.extend(produced.keys().cloned());
    let mut edges: BTreeMap<String, BTreeSet<(String, bool)>> = BTreeMap::new();
    for r in &prog.rules {
        for l in &r.body {
            if let Some(a) = l.atom().filter(|a| nodes.contains(&a.rel)) {
                edges.entry(r.head.rel.clone()).or_default().insert((a.rel.clone(), (a.neg || matches!(l, Literal::Agg { .. })) && !a.gated));
            }
        }
    }
    for (p, req) in &produced {
        edges.entry(p.clone()).or_default().insert((req.clone(), false));
    }
    let sccs = tarjan(&nodes, &edges);
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (r.as_str(), i))).collect();
    let answered: BTreeSet<usize> = prog.answers.keys().filter_map(|r| comp.get(r.as_str()).copied()).filter(|&c| sccs[c].iter().any(|r| produced.contains_key(r))).collect();
    if answered.is_empty() {
        return Ok(());
    }
    // Each gate: its `done` relation, its `ask` (none: the whole relation's gate, `done_N(0)`), the negated relation,
    // and the key's columns.
    let (mut rules, mut made) = (Vec::new(), Vec::<(String, Option<String>, String, Vec<(String, String)>)>::new());
    for r in std::mem::take(&mut prog.rules) {
        let Some(&c) = comp.get(r.head.rel.as_str()).filter(|c| answered.contains(c)) else {
            rules.push(r);
            continue;
        };
        let mut body: Vec<Literal> = Vec::new();
        for (i, l) in r.body.iter().enumerate() {
            let inside = l.atom().is_some_and(|a| comp.get(a.rel.as_str()) == Some(&c));
            if inside && matches!(l, Literal::Agg { .. }) {
                return Err(Error::NotStratifiable {
                    message: format!("{} aggregates {} inside a component that loads (decision 0065 §2a): not supported", r.head.rel, l.atom().map_or("?", |a| a.rel.as_str())),
                    component: sccs[c].clone(),
                    negated: l.atom().map_or_else(String::new, |a| a.rel.clone()),
                });
            }
            let Some(a) = l.atom().filter(|a| inside && a.neg && !a.gated) else {
                body.push(l.clone());
                continue;
            };
            // The key: the negated atom's columns that are not wild, all bound by the literals before it.
            let at: Vec<usize> = (0..a.args.len()).filter(|&k| !matches!(a.args[k], Term::Wild)).collect();
            let ad: String = (0..a.args.len()).map(|k| if at.contains(&k) { 'b' } else { 'f' }).collect();
            let key: Vec<Term> = at.iter().map(|&k| a.args[k].clone()).collect();
            let cols: Vec<(String, String)> = at.iter().map(|&k| (format!("c{k}"), prog.decls.get(&a.rel).and_then(|d| d.get(k)).map_or("int".to_owned(), |(_, t)| t.clone()))).collect();
            // Keyed when the literals before the negation read the component, so its keys grow with it; otherwise the
            // keys are fixed before it runs, and the whole relation's gate holds them all (as demand's does).
            let asks_inside = r.body[..i].iter().filter_map(Literal::atom).any(|b| comp.get(b.rel.as_str()) == Some(&c));
            if at.is_empty() || !asks_inside {
                let done = format!("done_{}", a.rel);
                if !made.iter().any(|(d, ..)| *d == done) {
                    made.push((done.clone(), None, a.rel.clone(), vec![("k".to_owned(), "int".to_owned())]));
                }
                body.push(Literal::Atom(atom(done, vec![Term::Int(0)])));
            } else {
                let (ask, done) = (format!("ask_{}_{ad}", a.rel), format!("done_{}_{ad}", a.rel));
                // The keys asked about: the rule's literals before the negation.
                rules.push(Rule { head: atom(ask.clone(), key.clone()), body: r.body[..i].to_vec() });
                if !made.iter().any(|(d, ..)| *d == done) {
                    made.push((done.clone(), Some(ask), a.rel.clone(), cols));
                }
                body.push(Literal::Atom(atom(done, key)));
            }
            body.push(Literal::Atom(Atom { neg: true, gated: true, ..a.clone() }));
        }
        rules.push(Rule { head: r.head, body });
    }
    // The gates: each `done` is fed from its `ask` once what its relation reads inside the component is quiet.
    let names: BTreeSet<String> = made.iter().map(|(d, ..)| d.clone()).collect();
    let mut gates = Vec::new();
    for (done, ask, rel, cols) in &made {
        prog.decls.insert(done.clone(), cols.clone());
        // Placement only: it puts `done` in the component (`native_stratum` drops a gate's rule).
        match ask {
            Some(ask) => {
                prog.decls.insert(ask.clone(), cols.clone());
                let vs: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("_g{i}"))).collect();
                rules.push(Rule { head: atom(done.clone(), vs.clone()), body: vec![Literal::Atom(atom(ask.clone(), vs))] });
            }
            None => {
                let wild = vec![Term::Wild; prog.decls.get(rel).map_or(0, Vec::len)];
                rules.push(Rule { head: atom(done.clone(), vec![Term::Int(0)]), body: vec![Literal::Atom(atom(rel.clone(), wild))] });
            }
        }
    }
    // What each relation reads inside its component, with the answers' edges, but not through a gate.
    let mut reads: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &rules {
        for l in &r.body {
            if let Some(a) = l.atom().filter(|a| !(a.neg && a.gated) && !names.contains(&a.rel)) {
                reads.entry(r.head.rel.clone()).or_default().insert(a.rel.clone());
            }
        }
    }
    for (p, req) in &produced {
        reads.entry(p.clone()).or_default().insert(req.clone());
    }
    for (done, ask, rel, _) in &made {
        let rel = rel.clone();
        let c = comp[rel.as_str()];
        let (mut reach, mut todo) = (BTreeSet::new(), vec![rel.clone()]);
        while let Some(v) = todo.pop() {
            if comp.get(v.as_str()) == Some(&c) || v.starts_with("ask_") {
                if reach.insert(v.clone()) {
                    todo.extend(reads.get(&v).into_iter().flatten().cloned());
                }
            }
        }
        let deps: Vec<String> = rules
            .iter()
            .filter(|r| reach.contains(&r.head.rel))
            .flat_map(|r| &r.body)
            .filter_map(|l| l.atom().map(|a| a.rel.clone()))
            .filter(|g| names.contains(g) && g != done)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        gates.push((done.clone(), Gate { src: ask.clone(), rel, level: 0, quiet: reach.into_iter().collect(), deps }));
    }
    // Levels: a gate whose relation reads another's comes after it.
    let level_edges: BTreeMap<String, BTreeSet<(String, bool)>> = gates.iter().map(|(d, g)| (d.clone(), g.deps.iter().map(|x| (x.clone(), false)).collect())).collect();
    let order = tarjan(&names, &level_edges);
    for (d, g) in &mut gates {
        g.level = order.iter().position(|s| s.contains(d.as_str())).unwrap_or(0);
    }
    prog.rules = rules;
    prog.gates.extend(gates);
    Ok(())
}

/// A program split at its requests: `load`, the rules every request depends on (its cone, gates and answers with
/// it), and `rest`, which reads what it needs of the cone as inputs (`reads`). The cone runs first, pausing for its
/// answers; the rest runs once over every document it loaded.
pub(in crate::code_rule_dsl) struct Loading {
    pub(in crate::code_rule_dsl) load: Program,
    pub(in crate::code_rule_dsl) rest: Program,
    pub(in crate::code_rule_dsl) reads: BTreeSet<String>,
}

/// [`Loading`], or `None` for a program without answers.
pub(in crate::code_rule_dsl) fn split(prog: &Program) -> Option<Loading> {
    if prog.answers.is_empty() {
        return None;
    }
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut cone: BTreeSet<String> = prog.answers.keys().filter(|r| derived.contains(r.as_str())).cloned().collect();
    let mut work: Vec<String> = cone.iter().cloned().collect();
    while let Some(rel) = work.pop() {
        for r in prog.rules.iter().filter(|r| r.head.rel == rel) {
            for a in r.body.iter().filter_map(Literal::atom) {
                if derived.contains(a.rel.as_str()) && cone.insert(a.rel.clone()) {
                    work.push(a.rel.clone());
                }
            }
        }
    }
    let reads: BTreeSet<String> = prog
        .rules
        .iter()
        .filter(|r| !cone.contains(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom))
        .filter(|a| cone.contains(&a.rel))
        .map(|a| a.rel.clone())
        .collect();
    let mut load = prog.clone();
    load.rules.retain(|r| cone.contains(&r.head.rel));
    load.outputs.retain(|o| cone.contains(o) || reads.contains(o));
    for r in &reads {
        if !load.outputs.contains(r) {
            load.outputs.push(r.clone());
        }
    }
    load.lattice.retain(|rel, _| cone.contains(rel));
    load.gates.retain(|(g, _)| cone.contains(g));
    let mut rest = prog.clone();
    rest.rules.retain(|r| !cone.contains(&r.head.rel));
    rest.outputs.retain(|o| !cone.contains(o));
    rest.lattice.retain(|rel, _| !cone.contains(rel));
    rest.gates.retain(|(g, _)| !cone.contains(g));
    rest.answers.clear();
    for r in &reads {
        if !rest.inputs.contains(r) {
            rest.inputs.push(r.clone());
        }
    }
    Some(Loading { load, rest, reads })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!seen(x)` inside a component that loads waits for its key's gate; the rule asking about it is its prefix.
    #[test]
    fn a_negation_inside_a_loading_component_is_gated_per_key() {
        let mut prog = super::super::rr::parse(
            ".decl want(x: symbol)\n.decl req(x: symbol)\n.decl got(x: symbol, y: symbol)\n.decl seen(x: symbol)\n.decl fresh(x: symbol)\n\
             want(\"a\").\nwant(y) :- got(_, y).\nseen(y) :- got(_, y).\nfresh(x) :- want(x), !seen(x).\nreq(x) :- fresh(x).\n",
        )
        .unwrap();
        prog.answers.insert("req".into(), vec!["got".into()]);
        assert!(super::super::front::strata(&prog).is_err(), "ungated, the negation is inside the component");
        gate(&mut prog).unwrap();
        let (sccs, _) = super::super::front::strata(&prog).unwrap();
        let loading = sccs.iter().find(|s| s.contains("req")).unwrap();
        for rel in ["got", "want", "fresh", "seen", "ask_seen_b", "done_seen_b"] {
            assert!(loading.contains(rel), "{rel} in {loading:?}");
        }
        let (done, g) = &prog.gates[0];
        assert_eq!((done.as_str(), g.src.as_deref(), g.rel.as_str()), ("done_seen_b", Some("ask_seen_b"), "seen"));
        assert!(g.quiet.contains(&"req".to_owned()), "a request unanswered keeps the gate shut: {:?}", g.quiet);
        prog.rules.push(super::super::rr::parse(".decl fresh(x: symbol)\n.decl shown(x: symbol)\nshown(x) :- fresh(x).\n").unwrap().rules.remove(0));
        prog.decls.insert("shown".into(), vec![("x".into(), "sym".into())]);
        let parts = split(&prog).unwrap();
        assert!(parts.load.rules.iter().all(|r| r.head.rel != "shown") && parts.rest.rules.iter().all(|r| r.head.rel == "shown"));
        assert_eq!(parts.reads.iter().collect::<Vec<_>>(), ["fresh"]);
        assert!(parts.rest.inputs.contains(&"fresh".to_owned()) && parts.rest.gates.is_empty() && parts.rest.answers.is_empty());
    }
}
