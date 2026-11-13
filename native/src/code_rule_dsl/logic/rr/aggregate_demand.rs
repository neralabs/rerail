//! Restrict private aggregate helpers to keys requested by their consumers.
//! This runs after relation/world lowering, so dependency checks see the complete graph.
use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

fn dependencies(p: &Program) -> BTreeMap<String, BTreeSet<String>> {
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &p.rules {
        deps.entry(r.head.rel.to_string()).or_default().extend(
            r.body
                .iter()
                .filter_map(Literal::atom)
                .map(|a| a.rel.to_string()),
        );
    }
    for (name, native) in &p.natives {
        deps.entry(name.clone())
            .or_default()
            .extend(native.reads().into_iter().map(str::to_owned));
    }
    deps
}

fn reaches(deps: &BTreeMap<String, BTreeSet<String>>, start: &str, target: &str) -> bool {
    let mut todo = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(at) = todo.pop() {
        if at == target {
            return true;
        }
        if seen.insert(at) {
            todo.extend(deps.get(at).into_iter().flatten().map(String::as_str));
        }
    }
    false
}

/// Safe positive guards, then comparisons whose inputs those guards bind. A context is a superset
/// of the consumer, never an assumption about a not-yet-computed aggregate or recursive result.
fn guards(
    rule: &Rule,
    helper: &str,
    keys: &BTreeSet<String>,
    deps: &BTreeMap<String, BTreeSet<String>>,
) -> Option<Vec<Literal>> {
    let safe: Vec<_> = rule
        .body
        .iter()
        .filter(|l| match l {
            Literal::Atom(a) => {
                !a.neg && !reaches(deps, &a.rel, helper) && !reaches(deps, &a.rel, &rule.head.rel)
            }
            Literal::Nearest { .. } => true,
            _ => false,
        })
        .cloned()
        .collect();
    // Do not introduce unrelated cross products when projecting a demand.
    let mut connected = keys.clone();
    loop {
        let before = connected.len();
        for l in &safe {
            let vars = l.vars();
            if !vars.is_disjoint(&connected) {
                connected.extend(vars);
            }
        }
        if connected.len() == before {
            break;
        }
    }
    let mut out: Vec<_> = safe
        .into_iter()
        .filter(|l| !l.vars().is_disjoint(&connected))
        .collect();
    let mut bound: BTreeSet<_> = out
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) => Some(a.vars()),
            _ => None,
        })
        .flatten()
        .collect();
    // Nearest cannot start with a free subject. All other variables must be grounded too.
    for l in &out {
        if let Literal::Nearest { n, a, .. } = l {
            if !bound.contains(n) {
                return None;
            }
            bound.insert(a.clone());
        }
    }
    let mut added = BTreeSet::new();
    loop {
        let before = added.len();
        for (i, l) in rule.body.iter().enumerate() {
            if added.contains(&i) {
                continue;
            }
            if let Literal::Cmp {
                l: left,
                op,
                r: right,
            } = l
            {
                if let Some(more) = front::cmp_binds(left, op, right, &bound) {
                    // Constants can test/alias a key; do not pull an aggregate's result into demand.
                    let vars = l.vars();
                    if vars.is_empty() || !vars.is_disjoint(&connected) {
                        bound.extend(more);
                        out.push(l.clone());
                        added.insert(i);
                    }
                }
            }
        }
        if added.len() == before {
            break;
        }
    }
    (keys.is_subset(&bound) && !out.is_empty()).then_some(out)
}

/// Whether `start` depends on `target` through `edges` (`front::strata`'s: a relation to those it reads).
fn depends(edges: &front::Edges, start: &str, target: &str) -> bool {
    let mut todo = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(at) = todo.pop() {
        if at == target {
            return true;
        }
        if seen.insert(at) {
            todo.extend(edges.get(at).into_iter().flatten().map(|(w, _)| w.as_str()));
        }
    }
    false
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Each candidate stratified whole, as before the shortcut (the test of the shortcut, which the engine hosts).
    pub static FULL_CHECKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn full_checks() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    return FULL_CHECKS.with(std::cell::Cell::get);
    #[cfg(not(any(test, feature = "test-hooks")))]
    false
}

pub fn optimize(program: &mut Program) {
    // The program's dependency edges while it stratifies. A candidate adds its demand relation, read by the
    // aggregate's rules, reading its guards' relations: a cycle it closes passes through the demand, so when no guard
    // depends on the aggregate the components are the program's and the demand's own, stratified as the program is,
    // and the candidate is not stratified again (one stratification a program, not one an aggregate).
    let mut known: Option<front::Edges> = front::strata(program).ok().map(|(_, edges)| edges);
    let names: BTreeSet<_> = program
        .rules
        .iter()
        .flat_map(|r| &r.body)
        .filter_map(|l| match l {
            Literal::Agg { atom, .. } if atom.rel.contains("rr_agg") => Some(atom.rel.to_string()),
            _ => None,
        })
        .collect();
    // Kept as candidates are placed, not made again for each (O-lower-aggregate-demand): the dependency map (each
    // placed candidate adds its demand's edges), the rules that read each candidate and the rules that make it (by
    // index; a placed candidate's demand rules join them), and whether the program is declared (a candidate adds a
    // declared demand relation, so it stays as it was, and only the demand rules' reads are checked). A candidate is
    // placed in the program itself, and taken out again where stratifying refuses it.
    let mut deps = dependencies(program);
    let declared = front::declared(program).is_ok();
    let mut readers: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut makers: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let index = |readers: &mut BTreeMap<String, Vec<usize>>, i: usize, rule: &Rule| {
        let mut seen = BTreeSet::new();
        for a in rule.body.iter().filter_map(Literal::atom) {
            if names.contains(a.rel.as_str()) && seen.insert(a.rel.as_str()) {
                readers.entry(a.rel.to_string()).or_default().push(i);
            }
        }
    };
    for (i, rule) in program.rules.iter().enumerate() {
        index(&mut readers, i, rule);
        if names.contains(rule.head.rel.as_str()) {
            makers.entry(rule.head.rel.to_string()).or_default().push(i);
        }
    }
    for name in names.iter().cloned() {
        if program.outputs.contains(&name)
            || program.inputs.contains(&name)
            || program.natives.contains_key(name.as_str())
            || program.facts.contains_key(name.as_str())
            || program.decls.contains_key(format!("rr_demand_agg_{name}").as_str())
        {
            continue;
        }
        let mut mask: Option<Vec<usize>> = None;
        let mut demands = Vec::new();
        let mut refused = false;
        for &ri in readers.get(&name).into_iter().flatten() {
            let rule = &program.rules[ri];
            for l in &rule.body {
                if l.atom().is_none_or(|a| a.rel != name) {
                    continue;
                }
                let Literal::Agg { atom, over, .. } = l else {
                    refused = true;
                    break;
                };
                let positions: Vec<_> = atom
                    .args
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| match t {
                        Term::Var(v) if Some(v.as_str()) != over.as_deref() => Some(i),
                        Term::Int(_) | Term::Str(_) => Some(i),
                        _ => None,
                    })
                    .collect();
                if positions.is_empty() || mask.as_ref().is_some_and(|m| *m != positions) {
                    refused = true;
                    break;
                }
                mask = Some(positions.clone());
                let args: Vec<_> = positions.iter().map(|&i| atom.args[i].clone()).collect();
                let keys = args
                    .iter()
                    .flat_map(|t| {
                        let mut v = BTreeSet::new();
                        t.vars(&mut v);
                        v
                    })
                    .collect();
                let Some(body) = guards(rule, &name, &keys, &deps) else {
                    refused = true;
                    break;
                };
                demands.push(Rule {
                    head: Atom {
                        rel: format!("rr_demand_agg_{name}").into(),
                        args: args.into(),
                        neg: false,
                        gated: false,
                    },
                    body: body.into(),
                });
            }
            if refused {
                break;
            }
        }
        if refused || demands.is_empty() {
            crate::code_rule_dsl::logic::profile::decision("aggregate demand", || {
                format!(
                    "{name}: unchanged; no safe complete consumer key or incompatible consumers"
                )
            });
            continue;
        }
        let positions = mask.unwrap();
        let demand = format!("rr_demand_agg_{name}");
        if !declared || demands.iter().any(|r| front::rule_declared(program, r).is_err()) {
            continue;
        }
        let guards: BTreeSet<String> = demands.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.to_string()).collect();
        let closes = known.as_ref().is_none_or(|edges| guards.iter().any(|g| depends(edges, g, &name))) || full_checks();
        // Placed: the demand declared, read first by the aggregate's rules, made by its demand rules.
        let decl = positions.iter().map(|&i| program.decls[name.as_str()][i].clone()).collect();
        program.decls.insert(demand.clone().into(), decl);
        for &mi in makers.get(&name).into_iter().flatten() {
            let rule = &mut program.rules[mi];
            let args = positions.iter().map(|&i| rule.head.args[i].clone()).collect();
            rule.body.insert(0, Literal::Atom(Atom { rel: demand.clone().into(), args, neg: false, gated: false }));
        }
        let first = program.rules.len();
        program.rules.extend(demands);
        if closes {
            match front::strata(program) {
                Ok((_, edges)) => known = Some(edges),
                Err(_) => {
                    // Refused: taken out again, the program as it was.
                    program.rules.truncate(first);
                    for &mi in makers.get(&name).into_iter().flatten() {
                        program.rules[mi].body.remove(0);
                    }
                    program.decls.remove(demand.as_str());
                    continue;
                }
            }
        } else {
            let edges = known.as_mut().expect("the edges of a stratified program");
            edges.entry(name.clone()).or_default().insert((demand.clone(), false));
            edges.entry(demand.clone()).or_default().extend(guards.iter().map(|g| (g.clone(), false)));
        }
        deps.entry(name.clone()).or_default().insert(demand.clone());
        deps.entry(demand.clone()).or_default().extend(guards);
        for i in first..program.rules.len() {
            index(&mut readers, i, &program.rules[i]);
        }
    }
    // Count zero is existence, provided its scalar result has no other consumer.
    // Native anti-joins and SQL NOT EXISTS probe the demanded group directly.
    for rule in &mut program.rules {
        let mut remove = BTreeSet::new();
        for i in 0..rule.body.len() {
            let Literal::Agg {
                var,
                op: AggOp::Count,
                over: None,
                atom,
            } = &rule.body[i]
            else {
                continue;
            };
            let uses: Vec<_> = rule
                .body
                .iter()
                .enumerate()
                .filter(|(j, l)| *j != i && l.vars().contains(var))
                .collect();
            if rule.head.vars().contains(var) || uses.len() != 1 {
                continue;
            }
            let (j, literal) = uses[0];
            let zero = matches!(literal, Literal::Cmp { l:Term::Int(0), op:"=", r:Term::Var(v) } | Literal::Cmp { l:Term::Var(v), op:"=", r:Term::Int(0) } if v==var);
            if !zero {
                continue;
            }
            let mut anti = atom.clone();
            anti.neg = true;
            rule.body.make_mut()[i] = Literal::Atom(anti);
            remove.insert(j);
        }
        let mut i = 0;
        rule.body.retain(|_| {
            let keep = !remove.contains(&i);
            i += 1;
            keep
        });
    }
}

// cratify: `tests` is rerail-logic's (it mounts it)

/// Whether `rel` is a helper the lowering makes (`rr_ctx…`, a context's `rr_cover…` groups, `rr_agg…`, and their
/// demand `rr_demand_agg_…`), under any program's prefix.
fn helper(rel: &str) -> bool {
    rel.contains("rr_ctx") || rel.contains("rr_cover") || rel.contains("rr_agg")
}

/// The helpers' columns typed again once the library's declarations are known (the combined program).
///
/// A program is lowered without the library: a helper column bound only by a library relation (`kind(n, …)`,
/// `child(p, n, _)`) finds no declaration then, and is a symbol by default. It holds nodes, which the library says;
/// and a column read through it (a demand's, `rr_demand_agg_…`, copied from the aggregate's) is then a symbol beside
/// the node the rule's own atoms make it, which no native plan compares. A column a symbol column binds stays one.
pub fn retype_helpers(program: &mut Program) {
    let mut changed = true;
    while changed {
        changed = false;
        let helpers: Vec<String> = program.decls.keys().filter(|r| helper(r)).cloned().map(String::from).collect();
        for rel in helpers {
            let width = program.decls[rel.as_str()].len();
            for i in 0..width {
                if program.decls[rel.as_str()][i].1 != "sym" {
                    continue;
                }
                let (mut found, mut symbol) = (None::<String>, false);
                for rule in program.rules.iter().filter(|r| r.head.rel == rel.as_str()) {
                    let Some(Term::Var(v)) = rule.head.args.get(i) else {
                        symbol = true;
                        continue;
                    };
                    for literal in &rule.body {
                        match literal {
                            Literal::Atom(a) if !a.neg => {
                                for (j, t) in a.args.iter().enumerate() {
                                    if !matches!(t, Term::Var(x) if x == v) {
                                        continue;
                                    }
                                    let ty = if front::is_axis(&a.rel) {
                                        Some("node".to_owned())
                                    } else {
                                        program.decls.get(a.rel.as_ref() as &str).and_then(|c| c.get(j)).map(|(_, t)| t.to_string())
                                    };
                                    match ty.as_deref() {
                                        Some("sym") if !helper(&a.rel) => symbol = true,
                                        Some("sym") | None => {}
                                        Some(t) => found = Some(t.to_owned()),
                                    }
                                }
                            }
                            Literal::Nearest { n, a, .. } if n == v || a == v => found = Some("node".to_owned()),
                            _ => {}
                        }
                    }
                }
                if let (Some(ty), false) = (found, symbol) {
                    program.decls.get_mut(rel.as_str()).expect("a helper").make_mut()[i].1 = ty.into();
                    changed = true;
                }
            }
        }
    }
}
