//! Compose validated SQL producers into shared scopes around actual native/deferred barriers.
use super::{Plan, Stage, quote};
use std::collections::{BTreeMap, BTreeSet};

const SQL_BYTES: usize = 1024 * 1024;

fn occurrences(query: &str, name: &str) -> usize {
    static IDENTIFIER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("SQL identifiers")
    });
    // The authorizer supplies the actual read set. Textual counts can overestimate
    // references in literals or local aliases, which only adds a conservative cut.
    IDENTIFIER
        .find_iter(query)
        .filter(|token| token.as_str().eq_ignore_ascii_case(name))
        .count()
        .max(1)
}

fn closure(
    needed: &BTreeSet<String>,
    stages: &BTreeMap<String, &Stage>,
    retained: &BTreeSet<String>,
    selected: &mut BTreeSet<String>,
    external: &mut BTreeSet<String>,
) {
    for name in needed {
        if !retained.contains(name)
            && let Some(stage) = stages.get(name)
        {
            if selected.insert(name.clone()) {
                closure(&stage.inputs, stages, retained, selected, external);
            }
        } else {
            external.insert(name.clone());
        }
    }
}

fn pure_projection(stage: &Stage, barriers: &BTreeSet<String>) -> bool {
    static PURE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        let identifier = r"[a-z_][a-z_0-9]*";
        let column = format!(r"(?:\*|{identifier}|NULL\s+AS\s+{identifier})");
        let select = format!(r"SELECT\s+{column}(?:\s*,\s*{column})*\s+FROM\s+{identifier}");
        regex::Regex::new(&format!(
            r"(?ix)^\s*{select}(?:\s+UNION\s+ALL\s+{select})*\s*$"
        ))
        .expect("pure SQL projections")
    });
    stage.inputs.is_disjoint(barriers) && PURE.is_match(&stage.query)
}

fn render(
    stages: &[Stage],
    selected: &BTreeSet<String>,
    query: &str,
    barriers: &BTreeSet<String>,
) -> String {
    if selected.is_empty() {
        return query.to_owned();
    }
    let definitions = stages
        .iter()
        .filter(|s| selected.contains(&s.name))
        .map(|s| {
            let mode = if pure_projection(s, barriers) {
                "NOT MATERIALIZED"
            } else {
                "MATERIALIZED"
            };
            format!("{} AS {mode} ({})", quote(&s.name), s.query)
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("WITH RECURSIVE {definitions} SELECT * FROM ({query})")
}

/// Every original stage was already validated, including stages unused by the result.
/// Shared ancestors crossing physical scopes become owners; multiple reads inside one scope
/// use one materialized CTE, except pure projections that can push column demand into stored inputs.
/// The chosen implementation has no execution configuration flag.
pub(in crate::code_rule_dsl) fn lower(plan: &mut Plan, final_reads: BTreeSet<String>) {
    // Native sources and deferred columns must retain their evaluation boundary.
    let mut barriers = plan
        .stages
        .iter()
        .filter(|s| !s.deferred.is_empty())
        .map(|s| s.name.clone())
        .collect::<BTreeSet<_>>();
    if let Some(syntax) = &plan.syntax {
        barriers.insert(syntax.name.clone());
        barriers.extend(syntax.edges.iter().cloned());
        barriers.extend(syntax.ownership.iter().cloned());
    }
    if plan.references.is_some() {
        barriers.insert("reference_edges".into());
    }
    let stages = plan
        .stages
        .iter()
        .map(|s| (s.name.clone(), s))
        .collect::<BTreeMap<_, _>>();
    // Physical scopes alone do not count repeated lookups inside a fused query.
    // Keep authored indexes when their producer has multiple validated reads.
    let mut readers = BTreeMap::<String, usize>::new();
    for stage in &plan.stages {
        for input in &stage.inputs {
            *readers.entry(input.clone()).or_default() += occurrences(&stage.query, input);
        }
    }
    for input in &final_reads {
        *readers.entry(input.clone()).or_default() += occurrences(&plan.query, input);
    }
    let mut retained = plan
        .stages
        .iter()
        .filter(|s| {
            (!s.indexes.is_empty() && readers.get(&s.name).is_some_and(|&n| n > 1))
                || s.reachability.is_some()
                || !s.deferred.is_empty()
                || s.recursive
                || s.query.to_ascii_uppercase().contains("COLLATE")
        })
        .map(|s| s.name.clone())
        .collect::<BTreeSet<_>>();
    loop {
        let mut scopes = Vec::new();
        let mut changed = false;
        for stage in &plan.stages {
            if !retained.contains(&stage.name) {
                continue;
            }
            if stage.reachability.is_some()
                && let Some(name) = stage
                    .inputs
                    .iter()
                    .find(|n| stages.contains_key(*n) && !retained.contains(*n))
            {
                retained.insert(name.clone());
                changed = true;
                break;
            }
            let mut inline = BTreeSet::new();
            let mut external = BTreeSet::new();
            closure(
                &stage.inputs,
                &stages,
                &retained,
                &mut inline,
                &mut external,
            );
            scopes.push((stage.query.as_str(), inline, &stage.inputs));
        }
        if changed {
            continue;
        }
        let mut inline = BTreeSet::new();
        let mut external = BTreeSet::new();
        closure(&final_reads, &stages, &retained, &mut inline, &mut external);
        scopes.push((plan.query.as_str(), inline, &final_reads));
        let mut uses = BTreeMap::<String, usize>::new();
        for (_, scope, _) in &scopes {
            for name in scope {
                *uses.entry(name.clone()).or_default() += 1;
            }
        }
        if let Some(stage) = plan
            .stages
            .iter()
            .rev()
            .find(|s| uses.get(&s.name).is_some_and(|&n| n > 1))
        {
            retained.insert(stage.name.clone());
            continue;
        }
        // AS MATERIALIZED controls execution, but SQLite still expands CTE references
        // during preparation. Bound the expanded producer graph as well as SQL text.
        let mut costs = BTreeMap::<String, usize>::new();
        for stage in &plan.stages {
            let mut cost = stage.query.len();
            for input in &stage.inputs {
                if !retained.contains(input) {
                    cost = cost.saturating_add(
                        occurrences(&stage.query, input)
                            .saturating_mul(*costs.get(input).unwrap_or(&0)),
                    );
                }
            }
            costs.insert(stage.name.clone(), cost);
        }
        let oversized = scopes
            .iter()
            .filter_map(|(query, selected, reads)| {
                let cost = reads.iter().filter(|name| selected.contains(*name)).fold(
                    query.len(),
                    |n, name| {
                        n.saturating_add(occurrences(query, name).saturating_mul(costs[name]))
                    },
                );
                (cost > SQL_BYTES).then_some((*query, selected, *reads))
            })
            .next();
        if let Some((query, selected, reads)) = oversized {
            let mut weights = reads
                .iter()
                .filter(|name| selected.contains(*name))
                .map(|n| (n.clone(), occurrences(query, n)))
                .collect::<BTreeMap<_, _>>();
            for stage in plan
                .stages
                .iter()
                .rev()
                .filter(|s| selected.contains(&s.name))
            {
                let weight = *weights.get(&stage.name).unwrap_or(&0);
                for input in stage.inputs.iter().filter(|name| selected.contains(*name)) {
                    let old = weights.entry(input.clone()).or_default();
                    *old =
                        old.saturating_add(weight.saturating_mul(occurrences(&stage.query, input)));
                }
            }
            if let Some(stage) = plan
                .stages
                .iter()
                .filter(|s| selected.contains(&s.name))
                .max_by_key(|s| costs[&s.name].saturating_mul(*weights.get(&s.name).unwrap_or(&0)))
            {
                retained.insert(stage.name.clone());
                continue;
            }
        }
        break;
    }
    let mut physical = Vec::new();
    let mut last = BTreeMap::new();
    for stage in &plan.stages {
        if !retained.contains(&stage.name) {
            continue;
        }
        let mut inline = BTreeSet::new();
        let mut external = BTreeSet::new();
        closure(
            &stage.inputs,
            &stages,
            &retained,
            &mut inline,
            &mut external,
        );
        let mut stage = stage.clone();
        stage.reads_source |= inline.iter().any(|n| stages[n].reads_source);
        stage.query = render(&plan.stages, &inline, &stage.query, &barriers);
        stage.inputs = external;
        for input in &stage.inputs {
            last.insert(input.clone(), physical.len());
        }
        physical.push(stage);
    }
    let mut inline = BTreeSet::new();
    let mut external = BTreeSet::new();
    closure(&final_reads, &stages, &retained, &mut inline, &mut external);
    plan.query = render(&plan.stages, &inline, &plan.query, &barriers);
    for input in external {
        last.insert(input, physical.len());
    }
    plan.last_uses = last;
    plan.stages = physical;
}
