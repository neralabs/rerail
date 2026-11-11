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

pub(super) fn optimize(program: &mut Program) {
    let names: BTreeSet<_> = program
        .rules
        .iter()
        .flat_map(|r| &r.body)
        .filter_map(|l| match l {
            Literal::Agg { atom, .. } if atom.rel.contains("rr_agg") => Some(atom.rel.to_string()),
            _ => None,
        })
        .collect();
    for name in names {
        if program.outputs.contains(&name)
            || program.inputs.contains(&name)
            || program.natives.contains_key(name.as_str())
            || program.facts.contains_key(name.as_str())
            || program.decls.contains_key(&format!("rr_demand_agg_{name}"))
        {
            continue;
        }
        let deps = dependencies(program);
        let mut mask: Option<Vec<usize>> = None;
        let mut demands = Vec::new();
        let mut refused = false;
        for rule in &program.rules {
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
                        Term::Var(v) if Some(v) != over.as_ref() => Some(i),
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
                        args,
                        neg: false,
                        gated: false,
                    },
                    body,
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
        let mut next = program.clone();
        let demand = format!("rr_demand_agg_{name}");
        next.decls.insert(
            demand.clone(),
            positions
                .iter()
                .map(|&i| program.decls[name.as_str()][i].clone())
                .collect(),
        );
        for rule in next.rules.iter_mut().filter(|r| r.head.rel == name) {
            rule.body.insert(
                0,
                Literal::Atom(Atom {
                    rel: demand.clone().into(),
                    args: positions
                        .iter()
                        .map(|&i| rule.head.args[i].clone())
                        .collect(),
                    neg: false,
                    gated: false,
                }),
            );
        }
        next.rules.extend(demands);
        if front::declared(&next).is_ok() && front::strata(&next).is_ok() {
            *program = next;
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
            rule.body[i] = Literal::Atom(anti);
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

#[cfg(test)]
mod tests;
