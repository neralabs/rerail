//! Physical SQL generation cuts large dependency expansions, then lets SQLite materialize the
//! remaining shared DAG. A few statement phases are optional experiment parameters, not API modes.
use super::{
    buffers,
    graph::{Boundary, Graph},
    lex,
};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(super) struct Plan {
    pub graph: Graph,
    pub queries: Vec<String>,
    pub metadata: Value,
}
pub(super) fn plan(
    graph: &Graph,
    outputs: &[(usize, usize)],
    expansion_budget: usize,
    phases: usize,
) -> Plan {
    plan_with_shape(
        graph,
        outputs,
        expansion_budget,
        phases,
        super::shapes::StageProbe::RowIn,
    )
}

pub(super) fn plan_with_shape(
    graph: &Graph,
    outputs: &[(usize, usize)],
    expansion_budget: usize,
    phases: usize,
    shape: super::shapes::StageProbe,
) -> Plan {
    let mut shaped = graph.clone();
    let rewritten = super::shapes::stage_probes(&mut shaped, shape);
    let anchors = super::shapes::ancestor_anchors(&mut shaped);
    let graph = &shaped;
    let mut reachable = HashSet::new();
    let mut pending: Vec<_> = outputs
        .iter()
        .map(|x| x.1)
        .chain(graph.barriers.iter().copied())
        .collect();
    while let Some(id) = pending.pop() {
        if reachable.insert(id) {
            pending.extend(graph.dependencies(&graph.relations[id].body));
        }
    }
    let mut expanded = vec![1usize; graph.relations.len()];
    let mut cuts = Vec::new();
    let mut max_body = 0;
    for (id, r) in graph
        .relations
        .iter()
        .enumerate()
        .filter(|(id, _)| reachable.contains(id))
    {
        let cost = lex::identifiers(&r.body)
            .into_iter()
            .filter_map(|word| {
                word.strip_prefix('q')
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&dep| dep < id)
            })
            .fold(1usize, |n, dep| n.saturating_add(expanded[dep]));
        max_body = max_body.max(cost);
        // Cut the expansion before SQLite duplicates an unbounded dependency diamond.
        if cost > expansion_budget {
            cuts.push(id);
            expanded[id] = 1;
        } else {
            expanded[id] = cost;
        }
    }
    if phases > 1 {
        loop {
            let width = (cuts.len() + 1).div_ceil(phases);
            let cut_set: HashSet<_> = cuts.iter().copied().collect();
            let mut uses = vec![HashSet::new(); graph.relations.len()];
            let roots = outputs
                .iter()
                .map(|x| x.1)
                .chain(graph.barriers.iter().copied())
                .collect();
            for (action, deps) in cuts
                .iter()
                .map(|&id| graph.dependencies(&graph.relations[id].body))
                .chain(std::iter::once(roots))
                .enumerate()
            {
                let mut pending: Vec<_> = deps.into_iter().collect();
                while let Some(id) = pending.pop() {
                    if cut_set.contains(&id) || !uses[id].insert(action / width) {
                        continue;
                    }
                    pending.extend(graph.dependencies(&graph.relations[id].body));
                }
            }
            let additions: Vec<_> = uses
                .iter()
                .enumerate()
                .filter(|(id, parts)| {
                    parts.len() > 1
                        && graph.relations[*id].boundary != Boundary::None
                        && !cut_set.contains(id)
                })
                .map(|(id, _)| id)
                .collect();
            if additions.is_empty() {
                break;
            }
            cuts.extend(additions);
            cuts.sort_unstable();
        }
    }
    let actions_per_phase = (cuts.len() + 1).div_ceil(phases.max(1));
    let (physical, queries) = buffers::render_plan(graph, outputs, &cuts, true, actions_per_phase);
    let metadata = json!({"ancestorAnchorRewrites":anchors,"stageProbeRewrites":rewritten,"stageProbe":format!("{shape:?}"),"nativeCuts":cuts,"sqliteMaterializedRelations":reachable.len()-cuts.len(),"maxProducerExpansion":max_body,"expansionBudget":expansion_budget,"sqlStatements":queries.len(),"requestedPhases":phases});
    Plan {
        graph: physical,
        queries,
        metadata,
    }
}

/// A single textual reference inside a subquery can execute once per outer row. Only plain
/// pipeline consumers qualify for inlining; identities and native batch boundaries stay pinned.
pub(super) fn materialize(graph: &Graph, id: usize) -> bool {
    let r = &graph.relations[id];
    if r.boundary != Boundary::None || r.uses != 1 {
        return true;
    }
    let name = Graph::name(id);
    graph.relations.iter().any(|consumer| {
        let body = &consumer.body;
        if !lex::identifiers(body).contains(&name.as_str()) {
            return false;
        }
        // A join can need an indexed intermediate even with one textual reference. Only
        // simple pipelines qualify; removing their index caused 69 million extra VM steps.
        if body.contains(" JOIN ")
            || body.contains(" GROUP BY ")
            || body.contains(" UNION ")
            || body.contains(" DISTINCT ")
        {
            return true;
        }
        let mut stack = Vec::new();
        let bytes = body.as_bytes();
        let mut quote = 0u8;
        let mut at = 0;
        while at < bytes.len() {
            let c = bytes[at];
            if quote != 0 {
                if c == quote {
                    if bytes.get(at + 1) == Some(&quote) {
                        at += 2;
                        continue;
                    }
                    quote = 0;
                }
            } else if c == b'\'' || c == b'"' {
                quote = c;
            } else if c == b'(' {
                stack.push(at);
            } else if c == b')' {
                stack.pop();
            } else if body[at..].starts_with(&name)
                && (at == 0 || !bytes[at - 1].is_ascii_alphanumeric())
                && bytes
                    .get(at + name.len())
                    .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
                && let Some(&open) = stack.last()
            {
                let head = body[..open].trim_end();
                if !(head.ends_with(" AS")
                    || head.ends_with("AS MATERIALIZED")
                    || head.ends_with("AS NOT MATERIALIZED"))
                {
                    return true;
                }
            }
            at += body[at..].chars().next().unwrap().len_utf8();
        }
        false
    })
}
