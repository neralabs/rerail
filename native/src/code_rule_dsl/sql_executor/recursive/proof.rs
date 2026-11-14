//! A partition is sound only when every recursive read stays in the head's partition.
use super::*;
struct Rule {
    output: String,
    head: Vec<E>,
    ports: Vec<(String, Vec<E>, Vec<(E, E)>)>,
}
fn predicates(block: &Select) -> Vec<&E> {
    block
        .predicates
        .iter()
        .chain(block.from.iter().flat_map(|i| match &i.join {
            Join::On(on) => on.iter(),
            Join::Cross => [].iter(),
        }))
        .collect()
}
fn collect(block: &Select, keys: &BTreeMap<String, usize>, ancestors: &[(E, E)], rule: &mut Rule) {
    let mut equalities = ancestors.to_vec();
    for e in predicates(block) {
        if let E::Binary {
            operator: B::Equal,
            left,
            right,
        } = e
            && plain(left) && plain(right) {
                equalities.push((left.as_ref().clone(), right.as_ref().clone()));
            }
    }
    for item in &block.from {
        if let Some(r) = item
            .relation
            .program_relation()
            .filter(|r| keys.contains_key(*r))
        {
            rule.ports.push((
                r.into(),
                (0..keys[r]).map(|i| field(&item.alias, i)).collect(),
                equalities.clone(),
            ));
        }
    }
    for e in block
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(predicates(block))
    {
        e.visit(&mut |e| {
            if let E::Scalar(q) | E::Exists { query: q, .. } = e {
                collect(q, keys, &equalities, rule);
            }
        });
    }
}
fn equivalent(a: &E, b: &E, equalities: &[(E, E)]) -> bool {
    let mut seen = vec![a];
    let mut at = 0;
    while at < seen.len() {
        let value = seen[at];
        if value == b {
            return true;
        }
        at += 1;
        for (left, right) in equalities {
            for (one, other) in [(left, right), (right, left)] {
                if one == value && !seen.contains(&other) {
                    seen.push(other);
                }
            }
        }
    }
    false
}

pub(super) fn targets(block: &Select, part: &E) -> Vec<E> {
    let equalities = predicates(block)
        .into_iter()
        .filter_map(|e| match e {
            E::Binary {
                operator: B::Equal,
                left,
                right,
            } if plain(left) && plain(right) => {
                Some((left.as_ref().clone(), right.as_ref().clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut result = vec![part.clone()];
    for (a, b) in &equalities {
        for e in [a, b] {
            if !result.contains(e) && equivalent(part, e, &equalities) {
                result.push(e.clone());
            }
        }
    }
    result
}
pub(super) fn partition(
    steps: &[(schedule::Step, Query)],
    keys: &BTreeMap<String, usize>,
) -> Option<BTreeMap<String, usize>> {
    if keys.values().any(|&n| n == 0) {
        return None;
    }
    let mut rules = Vec::new();
    for (step, q) in steps {
        let mut r = Rule {
            output: step.output.clone(),
            head: q
                .body
                .columns
                .iter()
                .map(|(e, _)| e.clone())
                .take(keys[&step.output])
                .collect(),
            ports: vec![],
        };
        collect(&q.body, keys, &[], &mut r);
        rules.push(r);
    }
    let mut domains = keys
        .iter()
        .map(|(r, &width)| {
            (
                r.clone(),
                (0..width)
                    .filter(|&col| {
                        rules
                            .iter()
                            .filter(|rule| rule.output == *r)
                            .all(|rule| plain(&rule.head[col]))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let constraints = rules
        .iter()
        .flat_map(|rule| {
            rule.ports.iter().map(|(input, columns, equalities)| {
                let pairs = rule
                    .head
                    .iter()
                    .map(|head| {
                        columns
                            .iter()
                            .map(|column| equivalent(head, column, equalities))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                (rule.output.clone(), input.clone(), pairs)
            })
        })
        .collect::<Vec<_>>();
    fn search(
        mut domains: BTreeMap<String, Vec<usize>>,
        constraints: &[(String, String, Vec<Vec<bool>>)],
    ) -> Option<BTreeMap<String, usize>> {
        // Propagate key-column constraints before choosing a column. Without this, an
        // impossible multi-relation component could require exponential blind guesses.
        loop {
            let mut changed = false;
            for (output, input, pairs) in constraints {
                let heads = domains[output].clone();
                let reads = domains[input].clone();
                let valid = |h: usize, i: usize| (output != input || h == i) && pairs[h][i];
                let next_heads = heads
                    .iter()
                    .copied()
                    .filter(|&h| reads.iter().any(|&i| valid(h, i)))
                    .collect::<Vec<_>>();
                let next_reads = reads
                    .iter()
                    .copied()
                    .filter(|&i| heads.iter().any(|&h| valid(h, i)))
                    .collect::<Vec<_>>();
                changed |= next_heads != heads || next_reads != reads;
                domains.insert(output.clone(), next_heads);
                domains.insert(input.clone(), next_reads);
            }
            if domains.values().any(Vec::is_empty) {
                return None;
            }
            if !changed {
                break;
            }
        }
        let Some((relation, choices)) = domains
            .iter()
            .filter(|(_, v)| v.len() > 1)
            .min_by_key(|(_, v)| v.len())
        else {
            return Some(domains.into_iter().map(|(r, v)| (r, v[0])).collect());
        };
        let relation = relation.clone();
        for &choice in choices {
            let mut next = domains.clone();
            next.insert(relation.clone(), vec![choice]);
            if let Some(solution) = search(next, constraints) {
                return Some(solution);
            }
        }
        None
    }
    if domains.values().any(Vec::is_empty) {
        return None;
    }
    search(std::mem::take(&mut domains), &constraints)
}
