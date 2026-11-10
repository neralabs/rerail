//! Adapt the logical language to executor-independent key and scalar proofs.
use super::{bags, keys, scalars};
mod storage;
use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Term};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct RuleAnalysis {
    pub head: String,
    pub uniqueness: keys::Proof,
    pub scalars: scalars::Analysis,
    pub contribution_storage: super::storage::Projection,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(in crate::code_rule_dsl) struct ProgramAnalysis {
    pub relation_keys: keys::Keys,
    pub key_inferences: Vec<KeyInference>,
    pub rules: Vec<RuleAnalysis>,
    pub scalar_storage: bags::Analysis,
    pub support_uniqueness: BTreeMap<String, super::Decision>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct KeyInference {
    pub relation: String,
    pub columns: Vec<usize>,
    pub reason: String,
    pub proof: Option<keys::Proof>,
}

fn variable(t: &Term) -> Option<String> {
    if let Term::Var(v) = t {
        Some(v.clone())
    } else {
        None
    }
}
fn equality(model: &mut keys::Projection, a: &Term, b: &Term) {
    if let Some(a) = variable(a) {
        match b {
            Term::Var(b) => model.equalities.push((a, b.clone())),
            Term::Int(_) | Term::Str(_) | Term::Top => {
                model.fixed.insert(a);
            }
            _ => {}
        }
    }
}
fn projection(head: &Atom, body: &[Literal], known: &keys::Keys) -> keys::Projection {
    let mut model = keys::Projection {
        outputs: head.args.iter().map(variable).collect(),
        ..Default::default()
    };
    for (i, literal) in body.iter().enumerate() {
        match literal {
            Literal::Atom(atom) if !atom.neg => {
                let columns: Vec<_> = (0..atom.args.len())
                    .map(|c| format!("input{i}.{c}"))
                    .collect();
                for (c, term) in atom.args.iter().enumerate() {
                    match term {
                        Term::Var(v) => model.equalities.push((columns[c].clone(), v.clone())),
                        Term::Int(_) | Term::Str(_) | Term::Top => {
                            model.fixed.insert(columns[c].clone());
                        }
                        Term::Call(_, _) => model.opaque = true,
                        _ => {}
                    }
                }
                model.inputs.push(keys::Input {
                    relation: atom.rel.clone(),
                    columns,
                    keys: known
                        .get(&atom.rel)
                        .cloned()
                        .unwrap_or_else(|| vec![(0..atom.args.len()).collect()]),
                });
            }
            Literal::Cmp { l, op: "=", r } => {
                equality(&mut model, l, r);
                equality(&mut model, r, l);
                if matches!(l, Term::Call(_, _)) || matches!(r, Term::Call(_, _)) {
                    model.opaque = true;
                }
            }
            Literal::Nearest { .. } | Literal::Ast { .. } => model.opaque = true,
            _ => {}
        }
    }
    model
}

fn scalar_inputs(body: &[Literal]) -> Vec<scalars::Scalar> {
    use scalars::{Expression as E, Kind};
    let mut outer: BTreeSet<_> = body
        .iter()
        .filter_map(|l| {
            if let Literal::Atom(a) = l {
                (!a.neg).then(|| a.vars())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    let aggregate_outputs: BTreeSet<_> = body
        .iter()
        .filter_map(|l| {
            if let Literal::Agg { var, .. } = l {
                Some(var.clone())
            } else {
                None
            }
        })
        .collect();
    let mut dependent_variables = aggregate_outputs.clone();
    outer.extend(aggregate_outputs.iter().cloned());
    loop {
        let before = (outer.len(), dependent_variables.len());
        for literal in body {
            let Literal::Cmp { l, op: "=", r } = literal else {
                continue;
            };
            for (a, b) in [(l, r), (r, l)] {
                let Term::Var(a) = a else { continue };
                let mut variables = BTreeSet::new();
                b.vars(&mut variables);
                if variables.is_subset(&outer) {
                    outer.insert(a.clone());
                    if matches!(b, Term::Call(_, _)) || !variables.is_disjoint(&dependent_variables)
                    {
                        dependent_variables.insert(a.clone());
                    }
                }
            }
        }
        if before == (outer.len(), dependent_variables.len()) {
            break;
        }
    }
    let variables: BTreeMap<_, _> = body
        .iter()
        .flat_map(Literal::vars)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, v)| (v, i))
        .collect();
    let mut result = vec![];
    for literal in body {
        let Literal::Agg { op, over, atom, .. } = literal else {
            continue;
        };
        let mut keys = vec![];
        let mut lookup = vec![];
        let mut predicates = vec![];
        let mut locals = BTreeMap::new();
        let mut value = E::Integer(1);
        let mut dependent = false;
        for (i, term) in atom.args.iter().enumerate() {
            match term {
                Term::Var(v) if Some(v) == over.as_ref() => {
                    if let Some(previous) = locals.insert(v, i) {
                        predicates.push(E::Apply(
                            "=".into(),
                            vec![E::Column(previous), E::Column(i)],
                        ));
                    }
                    value = E::Column(i);
                }
                Term::Var(v) if outer.contains(v) || aggregate_outputs.contains(v) => {
                    keys.push(E::Column(i));
                    lookup.push(E::Column(variables[v]));
                    dependent |= dependent_variables.contains(v);
                }
                Term::Var(v) => {
                    if let Some(previous) = locals.insert(v, i) {
                        predicates.push(E::Apply(
                            "=".into(),
                            vec![E::Column(previous), E::Column(i)],
                        ));
                    }
                }
                Term::Int(n) => {
                    predicates.push(E::Apply("=".into(), vec![E::Column(i), E::Integer(*n)]))
                }
                Term::Str(s) => {
                    predicates.push(E::Apply("=".into(), vec![E::Column(i), E::Text(s.clone())]))
                }
                Term::Wild => {}
                _ => predicates.push(E::Opaque(format!("{term:?}"))),
            }
        }
        if keys.is_empty() {
            keys.push(E::Integer(1));
        }
        predicates.sort();
        result.push(scalars::Scalar {
            kind: match op {
                AggOp::Count => Kind::Count,
                AggOp::Sum => Kind::Sum,
                AggOp::Product => Kind::Product,
                AggOp::Min => Kind::Min,
                AggOp::Max => Kind::Max,
                AggOp::Join { .. } => Kind::OrderedJoin,
            },
            source: atom.rel.clone(),
            keys,
            predicates,
            lookup,
            value,
            dependent,
            non_null: true,
        });
    }
    result
}

impl ProgramAnalysis {
    pub fn analyze(program: &front::Program) -> Self {
        let mut relation_keys: keys::Keys = program
            .decls
            .iter()
            .map(|(name, cols)| {
                let width = program.lattice.get(name).map_or(cols.len(), |l| l.keys);
                (name.clone(), vec![(0..width).collect()])
            })
            .collect();
        let mut key_inferences = relation_keys.iter().flat_map(|(name, keys)| keys.iter().map(|key| KeyInference {
            relation: name.clone(), columns: key.clone(),
            reason: if program.lattice.contains_key(name) { "A lattice has one value per declared key" } else { "A logical set is unique by its complete tuple; this does not prove unique rule witnesses" }.into(),
            proof: None,
        })).collect::<Vec<_>>();
        // Only a single rule without additional row producers can establish a narrower key. Counted once: a count
        // per rule per pass was quadratic in the rules (14,800 in a maintained program).
        let mut producers: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for r in &program.rules {
            *producers.entry(r.head.rel.as_str()).or_default() += 1;
        }
        let inputs: std::collections::HashSet<&str> = program.inputs.iter().map(String::as_str).collect();
        loop {
            let mut changed = false;
            for rule in &program.rules {
                let name = &rule.head.rel;
                if inputs.contains(name.as_str())
                    || program.facts.get(name).is_some_and(|v| !v.is_empty())
                    || program.natives.contains_key(name)
                    || program.lattice.contains_key(name)
                    || producers.get(name.as_str()) != Some(&1)
                {
                    continue;
                }
                let model = projection(&rule.head, &rule.body, &relation_keys);
                if let Some(key) = model.key() {
                    let current = relation_keys.entry(name.clone()).or_default();
                    if current.iter().all(|old| key.len() < old.len()) {
                        key_inferences.push(KeyInference { relation: name.clone(), columns: key.clone(), reason: "Projection preserves a determining input key, with one rule and no additional producer".into(), proof: Some(model.prove(&key)) });
                        *current = vec![key];
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let rules: Vec<RuleAnalysis> = program
            .rules
            .iter()
            .map(|r| RuleAnalysis {
                head: r.head.rel.clone(),
                uniqueness: projection(&r.head, &r.body, &relation_keys).unique(),
                scalars: scalars::analyze(scalar_inputs(&r.body)),
                contribution_storage: storage::analyze(r),
            })
            .collect();
        let scalar_storage = bags::analyze(rules.iter().enumerate().flat_map(|(rule, r)| {
            r.scalars.inputs.iter().cloned().enumerate().map(move |(scalar, input)| {
                (bags::Site { rule, head: r.head.clone(), scalar }, input)
            })
        }));
        let mut producers = BTreeMap::<String,Vec<_>>::new();
        for rule in &rules {
            producers.entry(rule.head.clone()).or_default().push((rule.uniqueness.decision.proven,rule.contribution_storage.heads.clone()));
        }
        for (rel,rows) in &program.facts {
            if !rows.is_empty() { producers.entry(rel.clone()).or_default().push((false,vec![])); }
        }
        let support_uniqueness = producers.into_iter().map(|(r,p)|(r,keys::union_support(&p))).collect();
        Self {
            support_uniqueness,
            scalar_storage,
            relation_keys,
            key_inferences,
            rules,
        }
    }

    pub fn explain(&self) {
        crate::code_rule_dsl::logic::profile::decision(
            "logical uniqueness and scalar summaries",
            || serde_json::to_string(self).expect("logical analysis is serializable"),
        );
    }
}
