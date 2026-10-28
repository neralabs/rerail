//! Independent executable specification for the complete async-call family.
//! This is deliberately hand-authored reference SQL, not a rule-ID compiler.
//! The generic compiler must later produce and qualify against this shape.
use super::{
    contract::{Coverage, FindingKey, Findings},
    shape::generated_shape,
};
use crate::code_rule_dsl::facts::producers::python::{
    call_batch,
    import_screen::Imports,
    python_facts::{self, Fact, NodeFacts, Qualification},
    python_structure::{ContextKind, NodeRef},
};
use crate::code_rule_dsl::facts::vtab::{self, Modules, Relation, Source};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    module_path::ModulePath,
    scope::{
        Reading, ScopeTable,
        options::{DEFAULT_PYTHON_MINOR, PythonOptions},
    },
    tape::TapeDoc,
};
use ast_grep_core::{Doc, Node};
use rusqlite::{Connection, StatementStatus};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    primary_refs: usize,
    admitted_primary_refs: usize,
    semantic_requests: usize,
    qualified_lookups: usize,
    binding_lookups: usize,
    candidates: usize,
    origin_rows: usize,
    argument_calls: usize,
    argument_rows: usize,
    arguments_ns: u64,
    sql_executions: usize,
    vm_steps: i32,
    fullscan_steps: i32,
    autoindex_rows: i32,
    preparation_ns: u64,
    diagnostics_ns: u64,
    diagnostics_wall_ns: u64,
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
/// S's bound `Relation`: the merged vtab's binding (whose `scans` counter is S's `filters`) and its rows.
struct Bound {
    binding: vtab::Binding,
    len: usize,
}
fn bind(modules: &Modules, db: &Connection, name: &str, relation: Relation) -> Bound {
    let len = relation.len();
    modules.bind(db, name, std::sync::Arc::new(relation)).unwrap();
    Bound { binding: modules.get(name).unwrap(), len }
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

/// Tape preorder IDs provide direct node access; no per-node Node map is built.
fn at<'t>(root: &Node<'t, TapeDoc>, pre: usize) -> Node<'t, TapeDoc> {
    root.root().adopt(root.get_doc().node(pre as u32))
}
/// Scope tables name targets by source span and kind. Tape starts are monotone
/// in preorder, so this locates a target without a global range-index copy.
fn target_at<'t>(
    root: &Node<'t, TapeDoc>,
    target: python_facts::Target,
) -> Option<Node<'t, TapeDoc>> {
    let tape = root.get_doc();
    let (mut lo, mut hi) = (0, tape.transducer_len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if tape.transducer_span(mid as u32).start < target.range.0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    while lo < tape.transducer_len() {
        let span = tape.transducer_span(lo as u32);
        if span.start != target.range.0 {
            break;
        }
        if span.end == target.range.1 && tape.transducer_kind(lo as u32) == target.kind {
            return Some(at(root, lo));
        }
        lo += 1;
    }
    None
}

/// Per-execution cache has one immutable tree/table/reading per document. It is
/// intentionally not a cross-revision cache: no unsafe range-only cache keys.
struct Semantics<'a, 't> {
    documents: &'a BTreeMap<u32, Node<'t, TapeDoc>>,
    modules: &'a BTreeMap<u32, ModulePath>,
    tables: BTreeMap<u32, ScopeTable>,
    facts: BTreeMap<(u32, usize), NodeFacts>,
    failed: BTreeMap<u32, String>,
}
impl<'a, 't> Semantics<'a, 't> {
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
                let root = self.documents[&doc].clone();
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
                ids.into_iter().map(|node| at(&self.documents[&doc], node)),
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
    fn qualified(
        &self,
        atoms: &mut Atoms,
        demand: impl IntoIterator<Item = (u32, usize)>,
    ) -> Relation {
        let mut relation = Relation::new(&["doc", "node", "symbol"]);
        for (doc, node) in demand {
            let Some(fact) = self.facts.get(&(doc, node)) else {
                continue;
            };
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
fn origins<'t>(
    owner: Node<'t, TapeDoc>,
    binding: Option<&python_facts::BindingProjection>,
    root: &Node<'t, TapeDoc>,
    with_context: bool,
) -> Vec<Node<'t, TapeDoc>> {
    let owner = unparen(owner);
    let mut out = vec![owner.clone()];
    let Some(binding) = binding else { return out };
    let find = |target: python_facts::Target| target_at(root, target);
    if let Some(value) = binding.value.and_then(find) {
        out.push(unparen(value));
    }
    if let Some(site) = binding.site.and_then(find) {
        if site.kind() == "assignment" {
            if let (Some(left), Some(right)) = (site.field("left"), site.field("right")) {
                let list = |node: &Node<'_, TapeDoc>| {
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

fn execute<'t>(
    roots: &[(u32, Node<'t, TapeDoc>)],
    modules: &BTreeMap<u32, ModulePath>,
    copies: usize,
) -> Run {
    execute_sql(roots, modules, copies, &[SELECT_TARGETS, COMPOSE])
}
fn execute_sql<'t>(
    roots: &[(u32, Node<'t, TapeDoc>)],
    modules: &BTreeMap<u32, ModulePath>,
    copies: usize,
    queries: &[&str; 2],
) -> Run {
    let started = cpu_ns();
    let wall = Instant::now();
    let mut metrics = Metrics {
        documents: roots.len(),
        ..Metrics::default()
    };
    let documents: BTreeMap<_, _> = roots.iter().cloned().collect();
    assert_eq!(documents.len(), roots.len(), "document IDs must be unique");
    let node = |doc, pre| at(&documents[&doc], pre);
    let mut imports: BTreeMap<u32, Imports> = BTreeMap::new();
    let structure = call_batch::collect(
        roots,
        ContextKind::AsyncFunction,
        |_, _| {},
        |doc, node| {
            imports
                .entry(doc)
                .or_default()
                .observe(node, modules.get(&doc));
        },
    );
    let parse_failed = roots
        .iter()
        .filter(|(_, root)| root.get_doc().syntax_error_nodes().next().is_some())
        .map(|(doc, _)| (*doc, "parse error".to_owned()))
        .collect();
    metrics.structure_ns = cpu_ns() - started;
    metrics.syntax_nodes = structure.counters.visited_nodes;
    metrics.calls = structure.counters.calls;
    let mut semantic = Semantics {
        documents: &documents,
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
            node(c.callee.doc, c.callee.node_id)
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
    let mut wanted_imports = BTreeSet::new();
    let mut wanted_globals = BTreeSet::new();
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
                wanted_imports.insert(name.as_str().unwrap().to_owned());
                wanted.push(&[
                    (label + 4 * copy) as i64,
                    atoms.get(name.as_str().unwrap()),
                    0,
                ]);
            }
            for name in direct["global"].as_array().into_iter().flatten() {
                wanted_globals.insert(name.as_str().unwrap().to_owned());
                wanted.push(&[
                    (label + 4 * copy) as i64,
                    atoms.get(&format!("builtin:{}", name.as_str().unwrap())),
                    0,
                ]);
            }
            if label == 1 {
                for name in utils["spawn"]["resolves"]["import"].as_array().unwrap() {
                    wanted_imports.insert(name.as_str().unwrap().to_owned());
                    wanted.push(&[
                        (label + 4 * copy) as i64,
                        atoms.get(name.as_str().unwrap()),
                        1,
                    ]);
                }
            }
        }
    }
    let no_imports = Imports::default();
    let primary: BTreeSet<_> = active.iter().filter_map(|site| site.ref_node).collect();
    metrics.primary_refs = primary.len();
    let admitted: BTreeSet<_> = primary
        .into_iter()
        .filter(|r| {
            imports.get(&r.doc).unwrap_or(&no_imports).may_qualify(
                &node(r.doc, r.node_id),
                &wanted_imports,
                &wanted_globals,
                modules.get(&r.doc),
            )
        })
        .collect();
    metrics.admitted_primary_refs = admitted.len();
    semantic.demand(admitted.iter().copied(), &mut metrics);
    let mut receivers = Relation::new(&["doc", "callee", "object"]);
    let active_ids: BTreeSet<_> = active
        .iter()
        .map(|s| (s.call.doc, s.report_node.node_id))
        .collect();
    let mut receiver_nodes = BTreeMap::new();
    for attr in &structure.attributes {
        if node(attr.name_node.doc, attr.name_node.node_id).text() == "open"
            && active_ids.contains(&(attr.node.doc, attr.node.node_id))
        {
            receivers.push(&[attr.node.doc as i64, id(attr.node), id(attr.object)]);
            receiver_nodes.insert((attr.node.doc, attr.node.node_id), attr.object);
        }
    }
    let mut sites = Relation::new(&["doc", "report", "call", "ref"]);
    for site in &active {
        if let Some(reference) = site.ref_node {
            if !admitted.contains(&reference)
                && !receiver_nodes.contains_key(&(site.call.doc, site.report_node.node_id))
            {
                continue;
            }
            sites.push(&[
                site.call.doc as i64,
                id(site.report_node),
                id(site.call),
                id(reference),
            ]);
        }
    }
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    let mut relations = Vec::new();
    for (name, relation) in [
        ("call_sites", sites),
        (
            "qualified",
            semantic.qualified(&mut atoms, admitted.iter().map(|r| (r.doc, r.node_id))),
        ),
        ("wanted_symbol", wanted),
        ("receiver", receivers),
    ] {
        relations.push((name, bind(&modules, &db, name, relation)));
    }
    let prep = cpu_ns();
    let mut query = db.prepare(queries[0]).unwrap();
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
    let argument_start = cpu_ns();
    let argument_demand: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate[4] == 1)
        .map(|candidate| {
            (
                candidate[1] as u32,
                node(candidate[1] as u32, candidate[3] as usize),
            )
        })
        .collect();
    let arguments = call_batch::collect_arguments(&argument_demand);
    metrics.argument_calls = arguments.counters.distinct_calls;
    metrics.argument_rows = arguments.counters.argument_rows;
    metrics.arguments_ns = cpu_ns() - argument_start;
    let calls: BTreeMap<_, _> = arguments
        .calls
        .iter()
        .map(|call| ((call.call.doc, id(call.call)), call))
        .collect();
    let mut seen_slots = BTreeSet::new();
    for c in &candidates {
        let doc = c[1] as u32;
        if c[4] == 1 && seen_slots.insert((doc, c[3])) {
            let call = calls[&(doc, c[3])];
            let args = &arguments.arguments[call.arguments.clone()];
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
            .map(|(r, _)| reference(r.doc, &unparen(node(r.doc, r.node_id)))),
        &mut metrics,
    );
    let mut origin_rows = Relation::new(&["doc", "owner", "reading", "value"]);
    let mut constructs = Relation::new(&["doc", "value", "callee"]);
    let mut value_demand = BTreeSet::new();
    let mut constructed = BTreeSet::new();
    for (owner, reading) in requests {
        let value_node = unparen(node(owner.doc, owner.node_id));
        let binding_fact = semantic
            .facts
            .get(&(owner.doc, value_node.node_id()))
            .filter(|f| {
                f.binding_eligible
                    && (reading == 1 || node(owner.doc, owner.node_id).kind() == "identifier")
            })
            .map(|f| {
                if reading == 1 {
                    &f.head_bindings.stop_at_global
                } else {
                    &f.head_bindings.default
                }
            });
        let binding = match binding_fact {
            Some(Fact::Known(binding)) => binding.as_ref(),
            Some(Fact::Unknown(reason) | Fact::Unsupported(reason)) => {
                semantic
                    .failed
                    .insert(owner.doc, format!("binding: {reason:?}"));
                continue;
            }
            _ => None,
        };
        for value in origins(value_node, binding, &documents[&owner.doc], reading == 0) {
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
    semantic.demand(value_demand.iter().copied(), &mut metrics);
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
        (
            "qualified_values",
            semantic.qualified(&mut atoms, value_demand.iter().map(|r| (r.doc, r.node_id))),
        ),
    ] {
        relations.push((name, bind(&modules, &db, name, relation)));
    }
    let p_wait = atoms.get("os.P_WAIT");
    let path = atoms.get("pathlib.Path");
    let prep = cpu_ns();
    let mut query = db.prepare(queries[1]).unwrap();
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
            rows.binding.counters().scans.load(std::sync::atomic::Ordering::Relaxed),
        );
        metrics.rows.insert(name.into(), rows.len);
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
                        let line = at(&documents[doc], *node).start_pos().line() + 1;
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
    let diagnostics_started = cpu_ns();
    let diagnostics_wall = Instant::now();
    for sql in queries {
        let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let mut rows = explain.raw_query();
        while let Some(row) = rows.next().unwrap() {
            metrics.explain.push(row.get(3).unwrap());
        }
    }
    metrics.diagnostics_ns = cpu_ns() - diagnostics_started;
    metrics.diagnostics_wall_ns = diagnostics_wall.elapsed().as_nanos() as u64;
    Run { findings, metrics }
}

#[cfg(test)]
mod native_oracle;
#[cfg(test)]
mod tests;
