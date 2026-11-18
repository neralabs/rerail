//! `.demand r, …` (T's `magic_sets`) on the tree, for `waves::policies` until its wave programs are versions: each
//! demanded relation restricted to the keys (first column) its consumers ask for. The program's demand, its
//! adornments and its gates are the version's (`plan::corpus::preparation::magic`, where this is `KeyDemand`).

use super::front::{Atom, Error, Literal, Program, Rule, Term, Value, components_of, join};
use std::collections::{BTreeMap, BTreeSet};

fn vars(t: &Term) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    t.vars(&mut out);
    out
}

fn is_const(t: &Term) -> bool {
    matches!(t, Term::Int(_) | Term::Branded(_) | Term::Str(_))
}

fn atom(rel: String, args: Vec<Term>) -> Atom {
    Atom { rel: rel.into(), args: args.into(), neg: false, gated: false }
}

fn magic_true() -> Literal {
    Literal::Atom(atom("magic_true".to_owned(), vec![Term::Int(0)]))
}

fn add_magic_true(prog: &mut Program) {
    prog.decls.insert("magic_true".to_owned().into(), [("k".into(), "int".into())].into());
    prog.facts.entry("magic_true".to_owned()).or_default().push([Value::Int(0)].into());
}

/// `.demand r, …`: applies demand, dropping a demanded relation whose negated use would close a negative cycle
/// through its own demand; that relation is computed in full (`demand_dropped` says why).
pub fn magic_sets(prog: Program) -> Result<Program, Error> {
    let mut dropped: Vec<(String, String)> = Vec::new();
    loop {
        let mut p = prog.clone();
        p.demand.retain(|r| !dropped.iter().any(|(d, _)| d == r));
        let demand = p.demand.clone();
        match magic_sets_once(p) {
            Ok(mut p) => {
                p.demand_dropped = dropped;
                return Ok(p);
            }
            Err(e) => {
                if let Some((_, negated)) = e.unstratified() {
                    if !demand.iter().any(|d| d == negated) {
                        return Err(e);
                    }
                    dropped.push((negated.to_owned(), "negative cycle through its demand".to_owned()));
                } else if let Error::UnboundDemand { relation, .. } = &e {
                    // Only a consequence of an earlier drop; otherwise the program asks for unbounded demand.
                    let Some((last, _)) = dropped.last() else { return Err(e) };
                    let why = format!("a use with an unbound key once {last} is computed in full");
                    dropped.push((relation.clone(), why));
                } else {
                    return Err(e);
                }
            }
        }
    }
}

/// Restricts each `.demand` relation to the keys its consumers ask for (first column bound). Every rule for a
/// demanded R gets `magic_R(key)` first; every use of a demanded S with its key bound produces `magic_S(key) :- <the
/// literals before it>`, a negated use too. Extras (negations, and atoms of the head's own recursive component) are
/// added greedily while the program stays stratifiable; leaving one out only widens demand.
fn magic_sets_once(mut prog: Program) -> Result<Program, Error> {
    if prog.demand.is_empty() {
        return Ok(prog);
    }
    let sccs = components_of(&prog)?;
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.as_str(), i))).collect();
    let demanded = |r: &str| prog.demand.iter().any(|d| d == r);
    let (mut rules, mut magic) = (Vec::new(), Vec::<(Atom, Vec<(Literal, bool)>)>::new());
    for rule in &prog.rules {
        let h = &rule.head;
        let demanded_head = demanded(&h.rel);
        let (mut seen, mut bound) = (Vec::new(), BTreeSet::new());
        if demanded_head {
            let key = h.args[0].clone();
            bound.extend(vars(&key));
            seen.push((Literal::Atom(atom(format!("magic_{}", h.rel), vec![key])), false));
        }
        for l in &rule.body {
            if let Literal::Atom(a) = l
                && demanded(&a.rel)
            {
                let key = &a.args[0];
                if !(vars(key).is_subset(&bound) || is_const(key)) {
                    return Err(Error::UnboundDemand {
                        message: format!("demand for {} is unbound in the rule for {}: [{}]", a.rel, h.rel, join(&rule.body)),
                        relation: a.rel.to_string(),
                    });
                }
                magic.push((atom(format!("magic_{}", a.rel), vec![key.clone()]), seen.clone()));
            }
            match l {
                Literal::Atom(a) if !a.neg => {
                    let same = demanded(&a.rel) && comp.get(a.rel.as_str()) == comp.get(h.rel.as_str());
                    seen.push((l.clone(), same));
                    bound.extend(a.vars());
                }
                Literal::Atom(a) if a.vars().is_subset(&bound) => seen.push((l.clone(), true)),
                Literal::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a) => {
                    seen.push((l.clone(), false));
                    bound.extend(l.vars());
                }
                Literal::Cmp { .. } if l.vars().is_subset(&bound) => seen.push((l.clone(), false)),
                _ => {}
            }
        }
        let mut body = Vec::new();
        if demanded_head {
            body.push(Literal::Atom(atom(format!("magic_{}", h.rel), vec![h.args[0].clone()])));
        }
        body.extend(rule.body.iter().cloned());
        rules.push(Rule { head: h.clone(), body: body.into() });
    }
    for r in prog.demand.clone() {
        let ty = prog.decls[r.as_str()][0].1.clone();
        prog.decls.insert(format!("magic_{r}").into(), [("k".into(), ty)].into());
    }
    add_magic_true(&mut prog);
    let mut keep: Vec<Vec<bool>> = magic.iter().map(|(_, pre)| pre.iter().map(|(_, extra)| !extra).collect()).collect();
    let build = |keep: &[Vec<bool>]| -> Result<Vec<Rule>, Error> {
        let mut out = Vec::new();
        for ((mh, pre), ks) in magic.iter().zip(keep) {
            // A kept literal still needs its variables bound by the kept literals before it.
            let (mut bound, mut body) = (BTreeSet::new(), Vec::new());
            for ((l, _), k) in pre.iter().zip(ks) {
                if !k {
                    continue;
                }
                match l {
                    Literal::Atom(a) if !a.neg => {
                        body.push(l.clone());
                        bound.extend(a.vars());
                    }
                    Literal::Nearest { n, a, .. } if bound.contains(n) || bound.contains(a) => {
                        body.push(l.clone());
                        bound.extend(l.vars());
                    }
                    _ if l.vars().is_subset(&bound) => body.push(l.clone()),
                    _ => {}
                }
            }
            if !vars(&mh.args[0]).is_subset(&bound) && !is_const(&mh.args[0]) {
                return Err(Error::UnboundDemand {
                    message: format!("demand for {} is unbound: [{}]", &mh.rel[6..], join(&pre.iter().map(|p| p.0.clone()).collect::<Vec<_>>())),
                    relation: mh.rel[6..].to_owned(),
                });
            }
            if body.is_empty() {
                body.push(magic_true());
            }
            out.push(Rule { head: mh.clone(), body: body.into() });
        }
        out.extend(rules.iter().cloned());
        Ok(out)
    };
    // Greedy: start from no extras (always stratifiable) and keep each extra that leaves the program stratifiable.
    prog.rules = build(&keep)?;
    components_of(&prog)?;
    for i in 0..magic.len() {
        for j in 0..magic[i].1.len() {
            if !magic[i].1[j].1 {
                continue;
            }
            keep[i][j] = true;
            prog.rules = build(&keep)?;
            match components_of(&prog) {
                Ok(_) => {}
                Err(e) if e.unstratified().is_some() => keep[i][j] = false,
                Err(e) => return Err(e),
            }
        }
    }
    prog.rules = build(&keep)?;
    prog.magic_extras = Some(
        magic.iter().zip(&keep).map(|((_, pre), ks)| pre.iter().zip(ks).filter(|((_, extra), k)| *extra && **k).count()).sum(),
    );
    Ok(prog)
}

// cratify: `tests` is rerail-logic's (it mounts it)
