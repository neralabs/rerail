//! Specialize a producer to a consumer's closed subtree domain. Keep other consumers independent.
use super::{
    ir::{self, Edge, KeyDemand, Program, Relation, Set},
    physical::{self, Predicate},
};
use std::collections::{BTreeSet, HashMap};

fn kinds(set: &Set, program: &Program, visiting: &mut BTreeSet<usize>) -> Option<BTreeSet<u16>> {
    match set {
        Set::Kinds(kinds) => Some(kinds.iter().copied().collect()),
        Set::Empty => Some(BTreeSet::new()),
        // Projected search roots satisfy the ancestor predicate even though origins remain distinct.
        Set::SearchRoots { filter, .. } => kinds(&Set::Reference(*filter), program, visiting),
        Set::Reference(id) if visiting.insert(*id) => {
            let result = kinds(&program.relations[*id].expression, program, visiting);
            visiting.remove(id);
            result
        }
        Set::Intersect(sets) => {
            let mut bound: Option<BTreeSet<u16>> = None;
            for set in sets {
                if let Some(next) = kinds(set, program, visiting) {
                    bound = Some(match bound {
                        None => next,
                        Some(old) => old.intersection(&next).copied().collect(),
                    });
                }
            }
            bound
        }
        Set::Union(sets) => {
            let mut bound = BTreeSet::new();
            for set in sets {
                bound.extend(kinds(set, program, visiting)?);
            }
            Some(bound)
        }
        _ => None,
    }
}

fn closed(program: &Program, producer: usize, roots: usize) -> Result<(), String> {
    let arms = physical::expand(&program.relations[producer].expression)?;
    for arm in arms {
        for (node, id) in &arm.references {
            if *id != producer {
                continue;
            }
            if arm.edges.iter().find(|(_, _, to)| to == node) != Some(&(0, Edge::Parent, *node)) {
                return Err("recursive dependency is not one parent hop".into());
            }
            let boundary = kinds(
                &program.relations[roots].expression,
                program,
                &mut BTreeSet::new(),
            )
            .ok_or("region root kinds are unknown")?;
            let guard = arm
                .predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Kinds(at, k) if at == node => {
                        Some(k.iter().copied().collect::<BTreeSet<_>>())
                    }
                    _ => None,
                })
                .reduce(|a, b| a.intersection(&b).copied().collect())
                .ok_or("recursive parent has no kind guard")?;
            if !guard.is_disjoint(&boundary) {
                return Err("recursive dependencies can cross the region boundary".into());
            }
        }
    }
    Ok(())
}

fn replace(set: &mut Set, old: usize, new: usize) {
    match set {
        Set::Reference(id) if *id == old => *id = new,
        Set::Intersect(sets) | Set::Union(sets) => {
            for set in sets {
                replace(set, old, new);
            }
        }
        Set::Complement(set) | Set::Preimage(_, set) => replace(set, old, new),
        _ => {}
    }
}

fn specialize(set: &mut Set, program: &mut Program, cache: &mut HashMap<(usize, usize), usize>) {
    match set {
        Set::Intersect(sets) => {
            // Flatten same-root conjunctions before demand propagation.
            let mut flat = Vec::new();
            for set in std::mem::take(sets) {
                match set {
                    Set::Intersect(inner) => flat.extend(inner),
                    other => flat.push(other),
                }
            }
            *sets = flat;
            for set in sets.iter_mut() {
                specialize(set, program, cache);
            }
            let domain = sets.iter().find_map(|set| match set {
                Set::Within(root) => Some(*root),
                _ => None,
            });
            if let Some(roots) = domain {
                for set in sets.iter_mut() {
                    let Set::Reference(id) = set else {
                        continue;
                    };
                    if program.relations[*id].domain.is_some() {
                        continue;
                    }
                    match closed(program, *id, roots) {
                        Ok(()) => {
                            let new=*cache.entry((*id,roots)).or_insert_with(|| {
                                let new=program.relations.len();
                                let mut expression=program.relations[*id].expression.clone();
                                replace(&mut expression,*id,new);
                                program.relations.push(Relation {
                                    source:format!("{} within r{roots}",program.relations[*id].source),
                                    expression,domain:Some(roots),keys:program.relations[*id].keys.clone(),
                                });
                                program.demand_decisions.push(format!("r{id} -> r{new}: seed keys restricted to r{roots}; recursive boundary is closed"));
                                new
                            });
                            *id = new;
                        }
                        Err(reason) => program
                            .demand_decisions
                            .push(format!("r{id}: output filter only for r{roots}: {reason}")),
                    }
                }
                // The specialized producer already satisfies this domain for every output key.
                if sets.iter().any(|set|matches!(set,Set::Reference(id) if program.relations[*id].domain==Some(roots))) {
                    sets.retain(|set|!matches!(set,Set::Within(id) if *id==roots));
                }
            }
            if sets.len() == 1 {
                *set = sets.pop().unwrap();
            }
        }
        Set::Union(sets) => {
            for set in sets {
                specialize(set, program, cache);
            }
        }
        // Do not infer positive demand from a complement or a different candidate node.
        _ => {}
    }
}

fn key_spec(program: &Program, producer: usize, roots: Vec<u16>) -> Result<KeyDemand, String> {
    let mut parents = BTreeSet::new();
    for arm in physical::expand(&program.relations[producer].expression)? {
        for (node, id) in &arm.references {
            if *id != producer {
                continue;
            }
            if arm.edges.iter().find(|(_, _, to)| to == node) != Some(&(0, Edge::Parent, *node)) {
                return Err("key demand needs one-parent recursive dependencies".into());
            }
            let guard = arm
                .predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Kinds(at, k) if at == node => {
                        Some(k.iter().copied().collect::<BTreeSet<_>>())
                    }
                    _ => None,
                })
                .reduce(|a, b| a.intersection(&b).copied().collect())
                .ok_or("key demand needs a recursive parent kind guard")?;
            parents.extend(guard);
        }
    }
    Ok(KeyDemand {
        root_kinds: roots,
        parent_kinds: parents.into_iter().collect(),
    })
}

fn keyed(set: &mut Set, program: &mut Program, cache: &mut HashMap<(usize, Vec<u16>), usize>) {
    match set {
        Set::Intersect(sets) => {
            let roots = kinds(&Set::Intersect(sets.clone()), program, &mut BTreeSet::new());
            for set in sets.iter_mut() {
                keyed(set, program, cache);
            }
            if let Some(roots) = roots.filter(|k| !k.is_empty()) {
                for set in sets {
                    let Set::Complement(inner) = set else {
                        continue;
                    };
                    let Set::Reference(id) = inner.as_mut() else {
                        continue;
                    };
                    if program.relations[*id].keys.is_some() {
                        continue;
                    }
                    let roots: Vec<_> = roots.iter().copied().collect();
                    if let Ok(spec) = key_spec(program, *id, roots.clone()) {
                        let new = *cache.entry((*id, roots)).or_insert_with(|| {
                            let new = program.relations.len();
                            let mut expression = program.relations[*id].expression.clone();
                            replace(&mut expression, *id, new);
                            program.relations.push(Relation {
                                source: format!(
                                    "{} for consumer kinds",
                                    program.relations[*id].source
                                ),
                                expression,
                                domain: None,
                                keys: Some(spec),
                            });
                            program.demand_decisions.push(format!(
                                "r{id} -> r{new}: candidate keys plus required parent dependencies"
                            ));
                            new
                        });
                        *id = new;
                    }
                }
            }
        }
        Set::Union(sets) => {
            for set in sets {
                keyed(set, program, cache);
            }
        }
        _ => {}
    }
}

pub(super) fn apply(program: &mut Program) {
    let mut keyed_cache = HashMap::new();
    for id in ir::reachable(program) {
        let mut expression = program.relations[id].expression.clone();
        keyed(&mut expression, program, &mut keyed_cache);
        program.relations[id].expression = expression;
    }
    let active = ir::reachable(program);
    let mut cache = HashMap::new();
    for id in active {
        let mut expression = program.relations[id].expression.clone();
        specialize(&mut expression, program, &mut cache);
        program.relations[id].expression = expression;
    }
}
