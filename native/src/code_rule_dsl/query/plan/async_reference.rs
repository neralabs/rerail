//! Independent executable specification for the complete async-call family.
//! This is deliberately hand-authored reference SQL, not a rule-ID compiler.
//! The generic compiler must later produce and qualify against this shape.
use super::{
    contract::{Coverage, FindingKey, Findings},
    python_structure::{self, ContextKind, NodeRef},
    relation::Relation,
};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::python_facts::{self, Fact, NodeFacts, Qualification},
    module_path::ModulePath,
    scope::{
        Reading, ScopeTable,
        options::{DEFAULT_PYTHON_MINOR, PythonOptions},
    },
};
use ast_grep_core::{Doc, Node};
use rusqlite::{Connection, StatementStatus};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::Instant,
};

const POLICIES: [&str; 4] = [
    include_str!(
        "../../../../../packs/python/rules/ruff-async210-blocking-http-call-in-async-function.json"
    ),
    include_str!(
        "../../../../../packs/python/rules/ruff-async221-run-process-in-async-function.json"
    ),
    include_str!(
        "../../../../../packs/python/rules/ruff-async222-wait-for-process-in-async-function.json"
    ),
    include_str!(
        "../../../../../packs/python/rules/ruff-async230-blocking-open-call-in-async-function.json"
    ),
];
const SELECT_TARGETS: &str = r#"
WITH sites AS MATERIALIZED (SELECT * FROM call_sites),
     refs AS MATERIALIZED (SELECT * FROM qualified),
     wanted AS MATERIALIZED (SELECT * FROM wanted_symbol),
     x AS MATERIALIZED (SELECT * FROM receiver)
SELECT DISTINCT w.label,s.doc,s.report,s.call,w.requirement
FROM wanted w JOIN refs r ON r.symbol=w.symbol
JOIN sites s ON s.doc=r.doc AND s.ref=r.node
UNION
SELECT 3,s.doc,s.report,s.call,2
FROM sites s JOIN x ON x.doc=s.doc AND x.callee=s.report
"#;
const COMPOSE: &str = r#"
WITH c AS MATERIALIZED (SELECT * FROM candidates),
     q AS MATERIALIZED (SELECT * FROM qualified_values),
     o AS MATERIALIZED (SELECT * FROM value_origin),
     a AS MATERIALIZED (SELECT * FROM call_slot),
     x AS MATERIALIZED (SELECT * FROM receiver),
     k AS MATERIALIZED (SELECT * FROM constructed)
SELECT label,doc,report FROM c WHERE requirement=0
UNION
SELECT c.label,c.doc,c.report FROM c
JOIN a ON a.doc=c.doc AND a.call=c.call
WHERE c.requirement=1 AND a.value=-1
UNION
SELECT c.label,c.doc,c.report FROM c
JOIN a ON a.doc=c.doc AND a.call=c.call
JOIN o ON o.doc=a.doc AND o.owner=a.value AND o.reading=1
JOIN q ON q.doc=o.doc AND q.node=o.value
WHERE c.requirement=1 AND q.symbol=?1
UNION
SELECT c.label,c.doc,c.report FROM c
JOIN x ON x.doc=c.doc AND x.callee=c.report
JOIN o ON o.doc=x.doc AND o.owner=x.object AND o.reading=0
JOIN k ON k.doc=o.doc AND k.value=o.value
JOIN q ON q.doc=k.doc AND q.node=k.callee
WHERE c.requirement=2 AND q.symbol=?2
"#;

#[derive(Default)]
struct Atoms(BTreeMap<String, i64>);
impl Atoms {
    fn get(&mut self, text: &str) -> i64 {
        let next = self.0.len() as i64;
        *self.0.entry(text.to_owned()).or_insert(next)
    }
}
#[derive(Default, Debug, serde::Serialize)]
struct Metrics {
    documents: usize,
    syntax_nodes: usize,
    calls: usize,
    async_calls: usize,
    scope_builds: usize,
    semantic_requests: usize,
    qualified_lookups: usize,
    binding_lookups: usize,
    candidates: usize,
    origin_rows: usize,
    sql_executions: usize,
    vm_steps: i32,
    fullscan_steps: i32,
    autoindex_rows: i32,
    preparation_ns: u64,
    execution_ns: u64,
    structure_ns: u64,
    scope_ns: u64,
    projection_ns: u64,
    sql_ns: u64,
    wall_ns: u64,
    filters: BTreeMap<String, u64>,
    rows: BTreeMap<String, usize>,
    explain: Vec<String>,
}
struct Run {
    findings: Findings,
    metrics: Metrics,
}
fn cpu_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid writable timespec; the process clock includes worker CPU.
    unsafe {
        libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time);
    }
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}
fn unparen<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    while node.kind() == "parenthesized_expression" {
        let Some(child) = node.children().find(|n| {
            n.is_named() && !n.kind().ends_with("comment") && n.kind() != "line_continuation"
        }) else {
            break;
        };
        node = child;
    }
    node
}
fn reference<'t, D: Doc>(doc: u32, node: &Node<'t, D>) -> NodeRef {
    NodeRef::of(doc, node)
}
fn id(node: NodeRef) -> i64 {
    node.node_id as i64
}

/// Per-execution cache has one immutable tree/table/reading per document. It is
/// intentionally not a cross-revision cache: no unsafe range-only cache keys.
struct Semantics<'a, 't, D: Doc> {
    nodes: &'a HashMap<(u32, usize), Node<'t, D>>,
    roots: &'a [(u32, Node<'t, D>)],
    modules: &'a BTreeMap<u32, ModulePath>,
    tables: BTreeMap<u32, ScopeTable>,
    facts: BTreeMap<(u32, usize), NodeFacts>,
    failed: BTreeMap<u32, String>,
}
impl<'a, 't, D: Doc> Semantics<'a, 't, D> {
    fn demand(&mut self, wanted: impl IntoIterator<Item = NodeRef>, metrics: &mut Metrics) {
        let started = cpu_ns();
        let scope_before = metrics.scope_ns;
        let mut requests: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
        for node in wanted {
            if !self.facts.contains_key(&(node.doc, node.node_id))
                && !self.failed.contains_key(&node.doc)
            {
                requests.entry(node.doc).or_default().insert(node.node_id);
            }
        }
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        for (doc, ids) in requests {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.tables.entry(doc) {
                let root = self
                    .roots
                    .iter()
                    .find(|(n, _)| *n == doc)
                    .unwrap()
                    .1
                    .clone();
                let scope_start = cpu_ns();
                let table = ScopeTable::build_in(
                    root,
                    GrammarDialect::Python,
                    10_000_000,
                    &|| false,
                    Reading::default(),
                    self.modules.get(&doc),
                );
                metrics.scope_ns += cpu_ns() - scope_start;
                match table {
                    Ok(table) => {
                        entry.insert(table);
                        metrics.scope_builds += 1;
                    }
                    Err(error) => {
                        self.failed.insert(doc, format!("scope: {error:?}"));
                        continue;
                    }
                }
            }
            let batch = python_facts::project(
                &self.tables[&doc],
                python_facts::Config {
                    module: self.modules.get(&doc),
                    python: &python,
                },
                ids.into_iter().map(|node| self.nodes[&(doc, node)].clone()),
            )
            .expect("fixed Python projection configuration");
            metrics.semantic_requests += batch.stats.distinct;
            metrics.qualified_lookups += batch.stats.qualified_lookups;
            metrics.binding_lookups += batch.stats.binding_lookups;
            for fact in batch.by_range.into_values().flatten() {
                if matches!(fact.qualification, Fact::Unknown(_) | Fact::Unsupported(_)) {
                    self.failed
                        .insert(doc, format!("qualification: {:?}", fact.qualification));
                }
                self.facts.insert((doc, fact.node_id), fact);
            }
        }
        metrics.projection_ns += cpu_ns() - started - (metrics.scope_ns - scope_before);
    }
    fn qualified(&self, atoms: &mut Atoms) -> Relation {
        let mut relation = Relation::new(&["doc", "node", "symbol"]);
        for (&(doc, node), fact) in &self.facts {
            let symbol = match &fact.qualification {
                Fact::Known(Qualification::Qualified { name, .. }) => Some(name.clone()),
                Fact::Known(Qualification::Unshadowed {
                    name,
                    builtin: true,
                    ..
                }) if fact.attributes.is_empty() => Some(format!("builtin:{name}")),
                _ => None,
            };
            if let Some(symbol) = symbol {
                relation.push(&[doc as i64, node as i64, atoms.get(&symbol)]);
            }
        }
        relation
    }
}

/// Shared structural value origins. These are node relations, never condition
/// evaluations. Constructors/symbols remain in SQL. One lookup per demanded owner.
fn origins<'t, D: Doc>(
    doc: u32,
    owner: Node<'t, D>,
    binding: Option<&python_facts::BindingProjection>,
    index: &HashMap<(u32, usize, usize, u16), Node<'t, D>>,
    with_context: bool,
) -> Vec<Node<'t, D>> {
    let owner = unparen(owner);
    let mut out = vec![owner.clone()];
    let Some(binding) = binding else { return out };
    let find = |target: python_facts::Target| {
        index
            .get(&(doc, target.range.0, target.range.1, target.kind))
            .cloned()
    };
    if let Some(value) = binding.value.and_then(find) {
        out.push(unparen(value));
    }
    if let Some(site) = binding.site.and_then(find) {
        if site.kind() == "assignment" {
            if let (Some(left), Some(right)) = (site.field("left"), site.field("right")) {
                let list = |node: &Node<'_, D>| {
                    matches!(
                        node.kind().as_ref(),
                        "list"
                            | "tuple"
                            | "set"
                            | "dictionary"
                            | "argument_list"
                            | "parameters"
                            | "lambda_parameters"
                            | "list_pattern"
                            | "tuple_pattern"
                            | "pattern_list"
                            | "expression_list"
                    )
                };
                // Catalog fallback: last direct name of this text and the value at
                // the same comma index. It intentionally includes set displays.
                let mut comma = 0;
                let mut target = None;
                for child in left.children() {
                    if child.kind() == "identifier" && child.text() == owner.text() {
                        target = Some(comma);
                    }
                    if !child.is_named() && child.text() == "," {
                        comma += 1;
                    }
                }
                if let Some(target) = target.filter(|_| list(&left) && list(&right)) {
                    let mut comma = 0;
                    for child in right.children() {
                        if child.is_named() && !child.kind().ends_with("comment") && comma == target
                        {
                            out.push(unparen(child.clone()));
                        }
                        if !child.is_named() && child.text() == "," {
                            comma += 1;
                        }
                    }
                }
            }
        }
        if with_context
            && site.kind() == "as_pattern"
            && site.parent().is_some_and(|p| p.kind() == "with_item")
        {
            let children: Vec<_> = site.children().collect();
            let plain = children
                .iter()
                .find(|n| n.kind() == "as_pattern_target")
                .and_then(|n| {
                    n.children()
                        .find(|n| n.is_named() && !n.kind().ends_with("comment"))
                })
                .is_some_and(|n| unparen(n).kind() == "identifier");
            if plain {
                for child in children.iter().take_while(|n| n.text() != "as") {
                    if child.is_named() && !child.kind().ends_with("comment") {
                        out.push(unparen(child.clone()));
                    }
                }
            }
        }
    }
    out.sort_by_key(Node::node_id);
    out.dedup_by_key(|n| n.node_id());
    out
}

fn execute<'t, D: Doc>(
    roots: &[(u32, Node<'t, D>)],
    modules: &BTreeMap<u32, ModulePath>,
    copies: usize,
) -> Run {
    let started = cpu_ns();
    let wall = Instant::now();
    let mut metrics = Metrics {
        documents: roots.len(),
        ..Metrics::default()
    };
    let mut nodes = HashMap::new();
    let mut ranges = HashMap::new();
    let mut parse_failed = BTreeMap::new();
    let structure = python_structure::collect_with(roots.to_vec(), |doc, node| {
        if node.is_error() || node.is_missing() {
            parse_failed.insert(doc, "parse error".into());
        }
        nodes.insert((doc, node.node_id()), node.clone());
        let r = node.range();
        ranges
            .entry((doc, r.start, r.end, node.kind_id()))
            .or_insert_with(|| node.clone());
    });
    metrics.structure_ns = cpu_ns() - started;
    metrics.syntax_nodes = structure.counters.visited_nodes;
    metrics.calls = structure.calls.len();
    let mut semantic = Semantics {
        nodes: &nodes,
        roots,
        modules,
        tables: BTreeMap::new(),
        facts: BTreeMap::new(),
        failed: parse_failed,
    };
    let active: Vec<_> = structure
        .calls
        .iter()
        .filter(|c| structure.contexts[c.context as usize].kind == ContextKind::AsyncFunction)
        .filter(|c| {
            nodes[&(c.callee.doc, c.callee.node_id)]
                .next()
                .is_some_and(|next| {
                    matches!(
                        next.kind().as_ref(),
                        "argument_list" | "generator_expression"
                    )
                })
        })
        .collect();
    metrics.async_calls = active.len();
    semantic.demand(active.iter().filter_map(|c| c.ref_node), &mut metrics);
    let mut atoms = Atoms::default();
    let mut wanted = Relation::new(&["label", "symbol", "requirement"]);
    for (label, text) in POLICIES.iter().enumerate() {
        let document: serde_json::Value = serde_json::from_str(text).unwrap();
        let utils = &document["clauses"][0]["forbid"]["ast"]["utils"];
        let direct = if label == 3 {
            &utils["target"]["any"][0]["resolves"]
        } else {
            &utils["target"]["resolves"]
        };
        for copy in 0..copies {
            for name in direct["import"].as_array().into_iter().flatten() {
                wanted.push(&[
                    (label + 4 * copy) as i64,
                    atoms.get(name.as_str().unwrap()),
                    0,
                ]);
            }
            for name in direct["global"].as_array().into_iter().flatten() {
                wanted.push(&[
                    (label + 4 * copy) as i64,
                    atoms.get(&format!("builtin:{}", name.as_str().unwrap())),
                    0,
                ]);
            }
            if label == 1 {
                for name in utils["spawn"]["resolves"]["import"].as_array().unwrap() {
                    wanted.push(&[
                        (label + 4 * copy) as i64,
                        atoms.get(name.as_str().unwrap()),
                        1,
                    ]);
                }
            }
        }
    }
    let mut sites = Relation::new(&["doc", "report", "call", "ref"]);
    for site in &active {
        if let Some(reference) = site.ref_node {
            sites.push(&[
                site.call.doc as i64,
                id(site.report_node),
                id(site.call),
                id(reference),
            ]);
        }
    }
    let mut receivers = Relation::new(&["doc", "callee", "object"]);
    let active_ids: BTreeSet<_> = active
        .iter()
        .map(|s| (s.call.doc, s.report_node.node_id))
        .collect();
    let mut receiver_nodes = BTreeMap::new();
    for attr in &structure.attributes {
        if nodes[&(attr.name_node.doc, attr.name_node.node_id)].text() == "open"
            && active_ids.contains(&(attr.node.doc, attr.node.node_id))
        {
            receivers.push(&[attr.node.doc as i64, id(attr.node), id(attr.object)]);
            receiver_nodes.insert((attr.node.doc, attr.node.node_id), attr.object);
        }
    }
    let db = Connection::open_in_memory().unwrap();
    let mut relations = Vec::new();
    for (name, relation) in [
        ("call_sites", sites),
        ("qualified", semantic.qualified(&mut atoms)),
        ("wanted_symbol", wanted),
        ("receiver", receivers),
    ] {
        relations.push((name, relation.bind(&db, name).unwrap()));
    }
    let prep = cpu_ns();
    let mut query = db.prepare(SELECT_TARGETS).unwrap();
    metrics.preparation_ns += cpu_ns() - prep;
    let sql_start = cpu_ns();
    let candidates: Vec<[i64; 5]> = query
        .query_map([], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?])
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    metrics.sql_ns += cpu_ns() - sql_start;
    metrics.sql_executions += 1;
    metrics.vm_steps += query.get_status(StatementStatus::VmStep);
    metrics.fullscan_steps += query.get_status(StatementStatus::FullscanStep);
    metrics.autoindex_rows += query.get_status(StatementStatus::AutoIndex);
    metrics.candidates = candidates.len();
    let mut requests: BTreeSet<(NodeRef, i64)> = BTreeSet::new();
    let mut slots = Relation::new(&["doc", "call", "value"]);
    let calls: BTreeMap<_, _> = structure
        .calls
        .iter()
        .map(|c| ((c.call.doc, id(c.call)), c))
        .collect();
    let mut seen_slots = BTreeSet::new();
    for c in &candidates {
        let doc = c[1] as u32;
        if c[4] == 1 && seen_slots.insert((doc, c[3])) {
            let call = calls[&(doc, c[3])];
            let args = &structure.arguments[call.arguments.clone()];
            let value = args
                .iter()
                .find(|a| a.keyword.as_deref() == Some("mode"))
                .or_else(|| args.iter().find(|a| a.positional_index == Some(0)))
                .map(|a| a.value);
            slots.push(&[c[1], c[3], value.map_or(-1, id)]);
            if let Some(value) = value {
                requests.insert((value, 1));
            }
        }
        if c[4] == 2 {
            requests.insert((receiver_nodes[&(doc, c[2] as usize)], 0));
        }
    }
    semantic.demand(
        requests
            .iter()
            .map(|(r, _)| reference(r.doc, &unparen(nodes[&(r.doc, r.node_id)].clone()))),
        &mut metrics,
    );
    let mut origin_rows = Relation::new(&["doc", "owner", "reading", "value"]);
    let mut constructs = Relation::new(&["doc", "value", "callee"]);
    let mut value_demand = BTreeSet::new();
    let mut constructed = BTreeSet::new();
    for (owner, reading) in requests {
        let node = unparen(nodes[&(owner.doc, owner.node_id)].clone());
        let binding = semantic
            .facts
            .get(&(owner.doc, node.node_id()))
            .filter(|f| {
                f.binding_eligible
                    && (reading == 1 || nodes[&(owner.doc, owner.node_id)].kind() == "identifier")
            })
            .and_then(|f| {
                let b = if reading == 1 {
                    &f.head_bindings.stop_at_global
                } else {
                    &f.head_bindings.default
                };
                if let Fact::Known(binding) = b {
                    binding.as_ref()
                } else {
                    None
                }
            });
        for value in origins(owner.doc, node, binding, &ranges, reading == 0) {
            let v = reference(owner.doc, &value);
            origin_rows.push(&[owner.doc as i64, id(owner), reading, id(v)]);
            if matches!(value.kind().as_ref(), "identifier" | "attribute") {
                value_demand.insert(v);
            }
            if reading == 0 && value.kind() == "call" {
                if let (Some(callee), Some(arguments)) =
                    (value.field("function"), value.field("arguments"))
                {
                    if matches!(callee.kind().as_ref(), "identifier" | "attribute")
                        && arguments.kind() == "argument_list"
                        && callee
                            .next()
                            .is_some_and(|next| next.node_id() == arguments.node_id())
                    {
                        let callee = reference(owner.doc, &callee);
                        value_demand.insert(callee);
                        if constructed.insert(v) {
                            constructs.push(&[owner.doc as i64, id(v), id(callee)]);
                        }
                    }
                }
            }
        }
    }
    metrics.origin_rows = origin_rows.len();
    semantic.demand(value_demand, &mut metrics);
    let mut candidate_rows = Relation::new(&["label", "doc", "report", "call", "requirement"]);
    for c in candidates {
        candidate_rows.push(&c);
        if c[4] == 2 {
            for copy in 1..copies {
                let mut c = c;
                c[0] += 4 * copy as i64;
                candidate_rows.push(&c);
            }
        }
    }
    for (name, relation) in [
        ("candidates", candidate_rows),
        ("call_slot", slots),
        ("value_origin", origin_rows),
        ("constructed", constructs),
        ("qualified_values", semantic.qualified(&mut atoms)),
    ] {
        relations.push((name, relation.bind(&db, name).unwrap()));
    }
    let p_wait = atoms.get("os.P_WAIT");
    let path = atoms.get("pathlib.Path");
    let prep = cpu_ns();
    let mut query = db.prepare(COMPOSE).unwrap();
    metrics.preparation_ns += cpu_ns() - prep;
    let sql_start = cpu_ns();
    let matches: Vec<(usize, u32, usize)> = query
        .query_map([p_wait, path], |r| {
            Ok((
                r.get::<_, i64>(0)? as usize,
                r.get(1)?,
                r.get::<_, i64>(2)? as usize,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    metrics.sql_ns += cpu_ns() - sql_start;
    metrics.sql_executions += 1;
    metrics.vm_steps += query.get_status(StatementStatus::VmStep);
    metrics.fullscan_steps += query.get_status(StatementStatus::FullscanStep);
    metrics.autoindex_rows += query.get_status(StatementStatus::AutoIndex);
    for (name, rows) in relations {
        metrics.filters.insert(
            name.into(),
            rows.filters.load(std::sync::atomic::Ordering::Relaxed),
        );
        metrics.rows.insert(name.into(), rows.len());
    }
    let mut by_domain: BTreeMap<(usize, u32), Vec<usize>> = BTreeMap::new();
    for (label, doc, node) in matches {
        by_domain.entry((label, doc)).or_default().push(node);
    }
    let mut findings = Findings::default();
    for copy in 0..copies {
        for (label, text) in POLICIES.iter().enumerate() {
            let policy: serde_json::Value = serde_json::from_str(text).unwrap();
            let rule = format!(
                "{}{}",
                policy["id"].as_str().unwrap(),
                if copy == 0 {
                    String::new()
                } else {
                    format!("-copy{copy}")
                }
            );
            let code = policy["clauses"][0]["violation"]["code"].as_str().unwrap();
            for (doc, _root) in roots {
                let coverage = semantic
                    .failed
                    .get(doc)
                    .map_or(Coverage::Complete, |reason| {
                        Coverage::Unknown(reason.clone())
                    });
                let keys = by_domain
                    .get(&(label + 4 * copy, *doc))
                    .into_iter()
                    .flatten()
                    .map(|node| {
                        let line = nodes[&(*doc, *node)].start_pos().line() + 1;
                        FindingKey {
                            rule: rule.clone(),
                            code: code.into(),
                            doc: *doc,
                            line,
                        }
                    });
                findings.publish(&rule, *doc, coverage, keys);
            }
        }
    }
    metrics.execution_ns = cpu_ns() - started - metrics.preparation_ns;
    metrics.wall_ns = wall.elapsed().as_nanos() as u64;
    // Diagnostics execute after the measured job and never invoke fact producers.
    for sql in [SELECT_TARGETS, COMPOSE] {
        let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let mut rows = explain.raw_query();
        while let Some(row) = rows.next().unwrap() {
            metrics.explain.push(row.get(3).unwrap());
        }
    }
    Run { findings, metrics }
}

#[cfg(test)]
mod tests;
