//! Explicit physical materialization into native buffers, inside one SQLite VM execution.
//! Producer aggregates are sequenced through a bounded recursive SQL scheduler. Consumers name buffers,
//! not producer CTEs, which bounds SQLite's recursive name-resolution expansion.
//! Native callbacks only publish rows; every combinator remains a SQL expression.
use super::{
    graph::{Boundary, Column, Graph, Policy},
    lex,
};
use std::collections::{HashMap, HashSet};

pub(super) fn render(graph: &Graph, outputs: &[(usize, usize)], policy: Policy) -> (Graph, String) {
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
    let cuts: Vec<_> = (0..graph.relations.len())
        .filter(|id| reachable.contains(id))
        .filter(|&id| {
            let r = &graph.relations[id];
            r.boundary != Boundary::None
                || policy == Policy::All
                || (policy == Policy::Reuse && r.uses > 1)
        })
        .collect();
    let (physical, mut queries) = render_plan(graph, outputs, &cuts, false, usize::MAX);
    (physical, queries.remove(0))
}

pub(super) fn render_plan(
    graph: &Graph,
    outputs: &[(usize, usize)],
    cuts: &[usize],
    materialize_inline: bool,
    max_actions: usize,
) -> (Graph, Vec<String>) {
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
    let substitutions: HashMap<_, _> = cuts
        .iter()
        .map(|&id| (Graph::name(id), format!("buffer_{id}")))
        .collect();
    // A buffer can be released after its last physical reader. Inline relations inherit
    // their transitive buffer reads; crossing another cut stops the walk.
    let cut_set: HashSet<_> = cuts.iter().copied().collect();
    let mut last_use: HashMap<_, _> = cuts.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    for (at, inputs) in cuts
        .iter()
        .map(|&id| graph.dependencies(&graph.relations[id].body))
        .chain(std::iter::once(
            outputs
                .iter()
                .map(|x| x.1)
                .chain(
                    graph
                        .barriers
                        .iter()
                        .copied()
                        .filter(|id| !cut_set.contains(id)),
                )
                .collect(),
        ))
        .enumerate()
    {
        let mut pending: Vec<_> = inputs.into_iter().collect();
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if cut_set.contains(&id) {
                last_use
                    .entry(id)
                    .and_modify(|last| *last = (*last).max(at));
            } else {
                pending.extend(graph.dependencies(&graph.relations[id].body));
            }
        }
    }
    let releases = |at: usize| {
        let mut names: Vec<_> = last_use
            .iter()
            .filter(|(_, last)| **last == at)
            .map(|(id, _)| format!("buffer_{id}"))
            .collect();
        names.sort();
        serde_json::to_string(&names).unwrap()
    };
    let mut physical = graph.clone();
    let mut definitions = Vec::new();
    for (id, r) in
        graph.relations.iter().enumerate().filter(|(id, _)| {
            reachable.contains(id) && !substitutions.contains_key(&Graph::name(*id))
        })
    {
        definitions.push(format!(
            "{}({}) AS {}({})",
            Graph::name(id),
            r.columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(","),
            if materialize_inline && super::physical::materialize(graph, id) {
                "MATERIALIZED"
            } else {
                "NOT MATERIALIZED"
            },
            lex::replace(&r.body, &substitutions)
        ));
    }
    let mut actions = Vec::new();
    for (at, &id) in cuts.iter().enumerate() {
        let r = &graph.relations[id];
        let handle = format!("buffer_{id}");
        physical.handles.push((handle.clone(), id));
        let cols = r
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let args = r
            .columns
            .iter()
            .map(|c| format!("p.{}", c.name))
            .collect::<Vec<_>>()
            .join(",");
        let body = lex::replace(&r.body, &substitutions);
        actions.push(format!("WHEN {} THEN CASE WHEN profile_begin({id}) THEN profile_end({id},(WITH producer({cols}) AS({body}) SELECT release_buffers(publish_{handle}({args}),'{}') FROM producer p)) END", actions.len(), releases(at)));
    }
    let force = graph
        .barriers
        .iter()
        .filter(|id| !cut_set.contains(id))
        .map(|id| {
            let name = substitutions
                .get(&Graph::name(*id))
                .cloned()
                .unwrap_or_else(|| Graph::name(*id));
            format!("(SELECT count(*) FROM {name})")
        })
        .collect::<Vec<_>>()
        .join("+");
    let force = if force.is_empty() {
        "0".to_owned()
    } else {
        force
    };
    let mut arms = vec![format!(
        "SELECT -1 AS clause,-1 AS file,-1 AS node,CAST({force} AS TEXT) AS captures"
    )];
    for &(clause, id) in outputs {
        let r = &graph.relations[id];
        let captures = r
            .columns
            .iter()
            .skip(2)
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let name = substitutions
            .get(&Graph::name(id))
            .cloned()
            .unwrap_or_else(|| Graph::name(id));
        arms.push(format!(
            "SELECT {clause},file,node,json_array({captures}) FROM {name}"
        ));
    }
    let body = if arms.len() > 128 {
        let mut names = Vec::new();
        for (i, chunk) in arms.chunks(128).enumerate() {
            let name = format!("result_chunk_{i}");
            definitions.push(format!(
                "{name}(clause,file,node,captures) AS NOT MATERIALIZED({})",
                chunk.join(" UNION ALL ")
            ));
            names.push(format!("SELECT * FROM {name}"));
        }
        names.join(" UNION ALL ")
    } else {
        arms.join(" UNION ALL ")
    };
    let columns = ["clause", "file", "node", "captures"]
        .iter()
        .map(|s| Column {
            name: (*s).into(),
            integer: *s != "captures",
        })
        .collect();
    let result = physical.add(columns, body.clone(), Boundary::Identity);
    physical.handles.push(("whole_result".into(), result));
    actions.push(format!("WHEN {} THEN CASE WHEN profile_begin({result}) THEN profile_end({result},(WITH producer(clause,file,node,captures) AS({body}) SELECT release_buffers(publish_whole_result(clause,file,node,captures),'{}') FROM producer)) END", actions.len(), releases(cuts.len())));
    let chunks = actions.chunks(max_actions.max(1));
    let total = chunks.len();
    let mut queries = Vec::new();
    for (part, chunk) in chunks.enumerate() {
        let first = part * max_actions.min(actions.len());
        let last = first + chunk.len();
        let mut local = definitions.clone();
        local.push(format!("schedule(step,token) AS MATERIALIZED(VALUES({},1) UNION ALL SELECT step+1,CASE step+1 {} END FROM schedule WHERE token IS NOT NULL AND step+1<{last})",first as i64-1,chunk.join(" ")));
        local.push(format!(
            "ready_final(token) AS MATERIALIZED(SELECT token FROM schedule WHERE step={})",
            last - 1
        ));
        let select = if part + 1 == total {
            "SELECT r.clause,r.file,r.node,r.captures FROM ready_final ready CROSS JOIN whole_result r ORDER BY r.clause,r.file,r.node"
        } else {
            "SELECT -1 AS clause,-1 AS file,-1 AS node,token AS captures FROM ready_final"
        };
        queries.push(format!("WITH RECURSIVE {} {select}", local.join(",\n")));
    }
    (physical, queries)
}
