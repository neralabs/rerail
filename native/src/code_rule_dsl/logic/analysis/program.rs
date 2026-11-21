//! Adapt the logical language to executor-independent key and scalar proofs.
use super::{bags, keys, scalars};
pub mod storage;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, CmpOp, Lit, LitRef, RuleRef, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuleAnalysis {
    pub head: String,
    pub uniqueness: keys::Proof,
    pub scalars: scalars::Analysis,
    pub contribution_storage: super::storage::Projection,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ProgramAnalysis {
    pub relation_keys: keys::Keys,
    pub key_inferences: Vec<KeyInference>,
    pub rules: Vec<RuleAnalysis>,
    pub scalar_storage: bags::Analysis,
    pub support_uniqueness: BTreeMap<String, super::Decision>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct KeyInference {
    pub relation: String,
    pub columns: Vec<usize>,
    pub reason: String,
    pub proof: Option<keys::Proof>,
}

/// A rule's variable's name.
fn name(v: &Version, r: RuleRef<'_>, s: crate::code_rule_dsl::logic::program_image::Slot) -> String {
    v.text(r.var(s)).to_owned()
}

/// The variables of term `t`, by name, into `out`.
fn term_vars(v: &Version, r: RuleRef<'_>, t: TermRef<'_>, out: &mut BTreeSet<String>) {
    match t {
        TermRef::Var(s) => {
            out.insert(name(v, r, s));
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(v, r, a, out)),
        _ => {}
    }
}

fn atom_vars(v: &Version, r: RuleRef<'_>, a: AtomRef<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    a.args().for_each(|t| term_vars(v, r, t, &mut out));
    out
}

/// The variables literal `l` names, by name.
fn lit_vars(v: &Version, r: RuleRef<'_>, l: LitRef<'_>) -> BTreeSet<String> {
    match l.view() {
        Lit::Atom(a) => atom_vars(v, r, a),
        Lit::Cmp { l, r: right, .. } => {
            let mut out = BTreeSet::new();
            term_vars(v, r, l, &mut out);
            term_vars(v, r, right, &mut out);
            out
        }
        Lit::Agg { var, over, atom, .. } => {
            let mut out = atom_vars(v, r, atom);
            out.insert(name(v, r, var));
            out.extend(over.map(|s| name(v, r, s)));
            out
        }
        Lit::Nearest { n, a, .. } => [name(v, r, n), name(v, r, a)].into(),
        Lit::Ast { document, node, captures, .. } => [document, node].into_iter().chain(captures.map(|(_, s)| s)).map(|s| name(v, r, s)).collect(),
    }
}

fn is_constant(t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) | TermRef::Top)
}

fn equality(v: &Version, r: RuleRef<'_>, model: &mut keys::Projection, a: TermRef<'_>, b: TermRef<'_>) {
    if let TermRef::Var(a) = a {
        match b {
            TermRef::Var(b) => model.equalities.push((name(v, r, a), name(v, r, b))),
            b if is_constant(b) => {
                model.fixed.insert(name(v, r, a));
            }
            _ => {}
        }
    }
}

fn projection(v: &Version, r: RuleRef<'_>, known: &keys::Keys) -> keys::Projection {
    let mut model = keys::Projection {
        outputs: r.head().args().map(|t| t.var().map(|s| name(v, r, s))).collect(),
        ..Default::default()
    };
    for (i, literal) in r.body().enumerate() {
        match literal.view() {
            Lit::Atom(atom) if !atom.neg() => {
                let columns: Vec<_> = (0..atom.arity()).map(|c| format!("input{i}.{c}")).collect();
                for (c, term) in atom.args().enumerate() {
                    match term {
                        TermRef::Var(s) => model.equalities.push((columns[c].clone(), name(v, r, s))),
                        t if is_constant(t) => {
                            model.fixed.insert(columns[c].clone());
                        }
                        TermRef::Call(_, _) => model.opaque = true,
                        _ => {}
                    }
                }
                let rel = v.rel_name(atom.rel());
                model.inputs.push(keys::Input {
                    relation: rel.to_owned(),
                    columns,
                    keys: known.get(rel).cloned().unwrap_or_else(|| vec![(0..atom.arity()).collect()]),
                });
            }
            Lit::Cmp { l, op: CmpOp::Eq, r: right } => {
                equality(v, r, &mut model, l, right);
                equality(v, r, &mut model, right, l);
                if matches!(l, TermRef::Call(_, _)) || matches!(right, TermRef::Call(_, _)) {
                    model.opaque = true;
                }
            }
            Lit::Nearest { .. } | Lit::Ast { .. } => model.opaque = true,
            _ => {}
        }
    }
    model
}

fn scalar_inputs(v: &Version, r: RuleRef<'_>) -> Vec<scalars::Scalar> {
    use scalars::{Expression as E, Kind};
    let body: Vec<LitRef<'_>> = r.body().collect();
    let mut outer: BTreeSet<String> = body
        .iter()
        .filter_map(|l| match l.view() {
            Lit::Atom(a) if !a.neg() => Some(atom_vars(v, r, a)),
            _ => None,
        })
        .flatten()
        .collect();
    let aggregate_outputs: BTreeSet<String> = body
        .iter()
        .filter_map(|l| match l.view() {
            Lit::Agg { var, .. } => Some(name(v, r, var)),
            _ => None,
        })
        .collect();
    let mut dependent_variables = aggregate_outputs.clone();
    outer.extend(aggregate_outputs.iter().cloned());
    loop {
        let before = (outer.len(), dependent_variables.len());
        for literal in &body {
            let Lit::Cmp { l, op: CmpOp::Eq, r: right } = literal.view() else {
                continue;
            };
            for (a, b) in [(l, right), (right, l)] {
                let TermRef::Var(a) = a else { continue };
                let a = name(v, r, a);
                let mut variables = BTreeSet::new();
                term_vars(v, r, b, &mut variables);
                if variables.is_subset(&outer) {
                    outer.insert(a.clone());
                    if matches!(b, TermRef::Call(_, _)) || !variables.is_disjoint(&dependent_variables) {
                        dependent_variables.insert(a);
                    }
                }
            }
        }
        if before == (outer.len(), dependent_variables.len()) {
            break;
        }
    }
    let variables: BTreeMap<String, usize> = body.iter().flat_map(|&l| lit_vars(v, r, l)).collect::<BTreeSet<_>>().into_iter().enumerate().map(|(i, v)| (v, i)).collect();
    let mut result = vec![];
    for literal in &body {
        let Lit::Agg { op, over, atom, .. } = literal.view() else {
            continue;
        };
        let over = over.map(|s| name(v, r, s));
        let mut keys = vec![];
        let mut lookup = vec![];
        let mut predicates = vec![];
        let mut locals = BTreeMap::new();
        let mut value = E::Integer(1);
        let mut dependent = false;
        for (i, term) in atom.args().enumerate() {
            match term {
                TermRef::Var(s) => {
                    let x = name(v, r, s);
                    if Some(&x) == over.as_ref() {
                        if let Some(previous) = locals.insert(x, i) {
                            predicates.push(E::Apply("=".into(), vec![E::Column(previous), E::Column(i)]));
                        }
                        value = E::Column(i);
                    } else if outer.contains(&x) || aggregate_outputs.contains(&x) {
                        keys.push(E::Column(i));
                        lookup.push(E::Column(variables[&x]));
                        dependent |= dependent_variables.contains(&x);
                    } else if let Some(previous) = locals.insert(x, i) {
                        predicates.push(E::Apply("=".into(), vec![E::Column(previous), E::Column(i)]));
                    }
                }
                TermRef::Int(n) => predicates.push(E::Apply("=".into(), vec![E::Column(i), E::Integer(n)])),
                TermRef::Branded(a) => predicates.push(E::Apply("=".into(), vec![E::Column(i), E::Integer(a.value())])),
                TermRef::Str(a) => predicates.push(E::Apply("=".into(), vec![E::Column(i), E::Text(v.text(a).to_owned())])),
                TermRef::Wild => {}
                other => predicates.push(E::Opaque(v.show_term(r, other))),
            }
        }
        if keys.is_empty() {
            keys.push(E::Integer(1));
        }
        // A bounded aggregate's value depends on its probe's bound (decision 0511): no summary of the group alone.
        if let AggRef::Bounded { bound, .. } = op {
            predicates.push(E::Opaque(format!("bounded by {}", name(v, r, bound))));
        }
        predicates.sort();
        result.push(scalars::Scalar {
            kind: match op {
                AggRef::Count => Kind::Count,
                AggRef::Sum => Kind::Sum,
                AggRef::Product => Kind::Product,
                AggRef::Min | AggRef::Bounded { max: false, .. } => Kind::Min,
                AggRef::Max | AggRef::Bounded { max: true, .. } => Kind::Max,
                AggRef::Join { .. } => Kind::OrderedJoin,
            },
            source: v.rel_name(atom.rel()).to_owned(),
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
    pub fn analyze(program: &Version) -> Self {
        let lattice = program.lattice_decls().unwrap_or_default();
        let natives = ProgramFields::natives(program);
        let mut relation_keys: keys::Keys = program
            .declared_names()
            .into_iter()
            .map(|name| {
                let width = lattice.get(name.as_str()).map_or_else(|| program.columns_of(&name).map_or(0, |c| c.len()), |l| l.keys);
                (name, vec![(0..width).collect()])
            })
            .collect();
        let mut key_inferences = relation_keys.iter().flat_map(|(name, keys)| keys.iter().map(|key| KeyInference {
            relation: name.clone(), columns: key.clone(),
            reason: if lattice.contains_key(name) { "A lattice has one value per declared key" } else { "A logical set is unique by its complete tuple; this does not prove unique rule witnesses" }.into(),
            proof: None,
        })).collect::<Vec<_>>();
        let rules: Vec<RuleRef<'_>> = program.rules().collect();
        let head = |r: RuleRef<'_>| program.rel_name(r.head().rel());
        // Only a single rule without additional row producers can establish a narrower key. Counted once: a count
        // per rule per pass was quadratic in the rules (14,800 in a maintained program).
        let mut producers: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for &r in &rules {
            *producers.entry(head(r)).or_default() += 1;
        }
        let inputs: std::collections::HashSet<String> = program.input_names().into_iter().collect();
        let with_facts: std::collections::HashSet<String> = program.fact_names().into_iter().filter(|rel| program.has_fact_rows(rel)).collect();
        // A rule's projection reads only its positive atoms' keys. A rule none of whose relations gained a key since
        // it was last projected gives the same key again, which its own relation (it is its only producer) already
        // holds: it is passed over, in the same sweeps and order, so the keys found are those every rule re-projected
        // in every sweep finds. Each sweep projected every rule (thousands, a sweep per step of a chain of keys).
        let mut step = 0usize;
        let mut gained: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut projected: Vec<Option<usize>> = vec![None; rules.len()];
        loop {
            let mut changed = false;
            for (ri, &rule) in rules.iter().enumerate() {
                let name = head(rule);
                if inputs.contains(name) || with_facts.contains(name) || natives.contains_key(name) || lattice.contains_key(name) || producers.get(name) != Some(&1) {
                    continue;
                }
                if let Some(at) = projected[ri]
                    && !rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && gained.get(program.rel_name(a.rel())).is_some_and(|&g| g > at)))
                {
                    continue;
                }
                projected[ri] = Some(step);
                let model = projection(program, rule, &relation_keys);
                if let Some(key) = model.key() {
                    let current = relation_keys.entry(name.to_owned()).or_default();
                    if current.iter().all(|old| key.len() < old.len()) {
                        key_inferences.push(KeyInference { relation: name.to_owned(), columns: key.clone(), reason: "Projection preserves a determining input key, with one rule and no additional producer".into(), proof: Some(model.prove(&key)) });
                        *current = vec![key];
                        changed = true;
                        step += 1;
                        gained.insert(name, step);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Each rule's analyses alone, on the pool's threads.
        use rayon::prelude::*;
        let rules: Vec<RuleAnalysis> = crate::code_rule_dsl::logic::profile::on_cores(|| {
            rules
                .par_iter()
                .map(|&r| RuleAnalysis {
                    head: head(r).to_owned(),
                    uniqueness: projection(program, r, &relation_keys).unique(),
                    scalars: scalars::analyze(scalar_inputs(program, r)),
                    contribution_storage: storage::analyze(program, r),
                })
                .collect()
        });
        let scalar_storage = bags::analyze(rules.iter().enumerate().flat_map(|(rule, r)| {
            r.scalars.inputs.iter().cloned().enumerate().map(move |(scalar, input)| {
                (bags::Site { rule, head: r.head.clone(), scalar }, input)
            })
        }));
        let mut producers = BTreeMap::<String,Vec<_>>::new();
        for rule in &rules {
            producers.entry(rule.head.clone()).or_default().push((rule.uniqueness.decision.proven,rule.contribution_storage.heads.clone()));
        }
        for rel in &with_facts {
            producers.entry(rel.clone()).or_default().push((false,vec![]));
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
