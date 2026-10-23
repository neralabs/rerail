//! `facts-a derived-check`: the corpus differential for SCHEMA.md § 2's views.
//!
//! Per file: build the facts, evaluate the views (`views::evaluate`). Unless `--no-check`, for
//! every batch-6 clause holding one of the shared stage texts (the text the most clauses hold
//! under that stage name), run the reference: the clause's own `nodes` with its gate and
//! ancestor predicate forced `true` (dump clauses) or its syntax-spec tables, then its stages up
//! to the targets, by the SQL engine. Each target stage's rows are compared with the view's rows
//! rendered into the stage's columns, as multisets and in order (the view's rows sorted by the
//! clause's `nodes` order of their key nodes, SCHEMA § 2.3).
use crate::eval::{self, Kind, Rule};
use crate::facts::{self, Facts, NONE};
use crate::rel::StagePlan;
use crate::repo::{summary, walk};
use crate::sql::engine::{self, Db, Table};
use crate::sql::value::V;
use crate::views::{self, Views};
use crate::{BATCH6, arg, call_config, policy, policy_doc};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// The oracle's gate-forced node tables (harness only: Spike A3's dump groups, moved here).
mod a3dump;
/// Stage name → view.
const TARGETS: &[(&str, &str)] = &[
    ("ownership", "py_owner"),
    ("functions", "py_function"),
    ("classes", "py_class"),
    ("methods", "py_method"),
    ("attributes", "py_attribute"),
    ("calls", "py_call"),
    ("parameters", "py_parameter"),
    ("assignments", "py_assignment"),
    ("decorators", "py_decorator"),
    ("import_identifiers", "py_import_identifier"),
    ("contexts", "py_context"),
];
const PROVIDER_PATH: &str = "impl/common/l2070_server_plan/backend_profiles.py";
/// SCHEMA § 2.4's composed views: the stage text as written, over the § 2.2 views rendered (in
/// place of the stages they stand for) and the clause's matcher inputs.
const COMPOSITE: &[&str] = &["string_values", "special_attributes", "return_kinds", "direct_events"];

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Target {
    rule: usize,
    clause: usize,
    clause_id: String,
    /// (stage index, stage name): stages a § 2.2 view stands for.
    stages: Vec<(usize, String)>,
    /// (stage index, stage name): § 2.4 composed views.
    composite: Vec<(usize, String)>,
    /// Stage indices to run, in order.
    run: Vec<usize>,
    /// Plan inputs the run reads outside the dump (matcher inputs), gate-free (`gate_free`).
    extra: Vec<usize>,
    /// Per `extra` input: its lowering by the pattern front end (`rerail_facts::front::pattern`),
    /// the view side of a § 2.4 composition (`view_inputs`).
    lowered: Vec<(usize, Result<rerail_facts::ir::Lowered, String>)>,
    /// The clause's dump group with its gate and ancestor predicate forced `true`.
    dump: Option<a3dump::DumpGroup>,
}

/// A matcher input's rule over all of `N`, as the dump's gate is forced `true`: its nested `all`s
/// flattened, without the conjuncts that read a selection utility (`matches`) or the path
/// (`filePath`; facts are path-free).
fn gate_free(rule: &Json) -> Json {
    fn flat(v: &Json, out: &mut Vec<Json>) {
        match v.as_object().and_then(|o| (o.len() == 1).then(|| o.get("all")).flatten()).and_then(Json::as_array) {
            Some(items) => items.iter().for_each(|x| flat(x, out)),
            None => out.push(v.clone()),
        }
    }
    fn mentions(v: &Json) -> bool {
        match v {
            Json::Object(o) => o.iter().any(|(k, x)| k == "matches" || k == "filePath" || mentions(x)),
            Json::Array(a) => a.iter().any(mentions),
            _ => false,
        }
    }
    let mut all = Vec::new();
    flat(rule, &mut all);
    json!({"all": all.into_iter().filter(|c| !mentions(c)).collect::<Vec<_>>()})
}

/// The shared text per target stage name over batch 6, and the clauses holding it.
fn shared_texts(policies: &str) -> BTreeMap<String, (String, Vec<(String, String)>)> {
    let event = json!({"production_scope": true});
    let mut by: BTreeMap<String, BTreeMap<String, Vec<(String, String)>>> = BTreeMap::new();
    for check in BATCH6 {
        let doc = policy_doc(policies, check);
        let (config, _) = call_config(&doc, &event);
        let r = policy::resolve(&doc, &config).unwrap();
        for c in r["clauses"].as_array().unwrap() {
            let Some(rel) = c.get("relational") else { continue };
            for st in rel.get("stages").and_then(Json::as_array).into_iter().flatten() {
                let name = st["name"].as_str().unwrap();
                if !TARGETS.iter().any(|(s, _)| *s == name) && !COMPOSITE.contains(&name) {
                    continue;
                }
                if let Some(q) = st.get("query").and_then(Json::as_str) {
                    by.entry(name.into()).or_default().entry(norm(q)).or_default().push((check.to_string(), c["id"].as_str().unwrap().to_owned()));
                }
            }
        }
    }
    by.into_iter().map(|(name, texts)| {
        let (t, cls) = texts.into_iter().max_by_key(|(_, v)| v.len()).unwrap();
        (name, (t, cls))
    }).collect()
}

fn true_query(rule: &mut Rule) -> crate::ast::Query {
    crate::ast::Compiler::new(&mut rule.names, &facts::GRAMMAR, &BTreeMap::new())
        .query(&json!({"language": "python", "syntax": "valid", "rule": {"all": []}}))
        .unwrap()
}

/// Batch 6 compiled as the suite's production event, gates forced `true` in target clauses. A
/// target that reads matcher inputs outside the dump gets them gate-free (`gate_free`): the rule
/// is resolved, those inputs' rules are replaced, and it is compiled again.
fn compile(policies: &str, shared: &BTreeMap<String, (String, Vec<(String, String)>)>) -> (Vec<Rule>, Vec<Target>) {
    let event = json!({"production_scope": true});
    let mut rules = Vec::new();
    let mut targets = Vec::new();
    for (ri, check) in BATCH6.iter().enumerate() {
        let doc = policy_doc(policies, check);
        let (config, scope) = call_config(&doc, &event);
        let resolved_doc = policy::resolve(&doc, &config).unwrap();
        let mut rule = eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR).unwrap();
        let mut found = rule_targets(&mut rule, &resolved_doc, ri, check, shared);
        let extra: BTreeMap<String, Vec<String>> = found
            .iter()
            .filter(|t| !t.extra.is_empty())
            .map(|t| {
                let Kind::Relational(plan) = &rule.clauses[t.clause].kind else { unreachable!() };
                (t.clause_id.clone(), t.extra.iter().map(|&pi| plan.inputs[pi].0.clone()).collect())
            })
            .collect();
        if !extra.is_empty() {
            let mut resolved = policy::resolve(&doc, &config).unwrap();
            for c in resolved["clauses"].as_array_mut().unwrap() {
                let Some(names) = c["id"].as_str().and_then(|id| extra.get(id)).cloned() else { continue };
                for n in names {
                    let r = &mut c["relational"]["inputs"][n.as_str()]["evidence"]["query"]["ast"]["rule"];
                    assert!(!r.is_null(), "{check}: input {n} has no ast rule");
                    *r = gate_free(r);
                }
            }
            rule = eval::Rule::compile(&resolved, &config, scope.as_ref(), &facts::GRAMMAR).unwrap();
            found = rule_targets(&mut rule, &resolved, ri, check, shared);
            // The same gate-free inputs through the pattern front end (S1), matched by evidence.
            let program = rerail_facts::compile::compile_report(&resolved, &config, &mut crate::leaf::Leaves::new()).map(|c| c.program);
            for t in found.iter_mut() {
                let Kind::Relational(plan) = &rule.clauses[t.clause].kind else { unreachable!() };
                let clause = resolved["clauses"].as_array().unwrap().iter().find(|c| c["id"] == t.clause_id.as_str()).unwrap();
                t.lowered = t
                    .extra
                    .iter()
                    .map(|&pi| {
                        let ast = &clause["relational"]["inputs"][plan.inputs[pi].0.as_str()]["evidence"]["query"]["ast"];
                        let lowered = match &program {
                            Ok(p) => p
                                .inputs
                                .iter()
                                .find(|d| matches!(&d.kind, rerail_facts::ir::InputKind::Pattern { query } if query == ast))
                                .map_or(Err("input not in the compiled program".to_string()), |d| d.algebra.clone()),
                            Err(e) => Err(format!("compile: {e}")),
                        };
                        (pi, lowered)
                    })
                    .collect();
            }
        }
        targets.extend(found);
        rules.push(rule);
    }
    (rules, targets)
}

fn rule_targets(rule: &mut Rule, resolved: &Json, ri: usize, check: &str, shared: &BTreeMap<String, (String, Vec<(String, String)>)>) -> Vec<Target> {
    let mut utils: BTreeMap<String, Json> = BTreeMap::new();
    for c in resolved["clauses"].as_array().into_iter().flatten() {
        if let Some(u) = c.get("define").and_then(|d| d.get("utils")).and_then(Json::as_object) {
            utils.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    let mut targets = Vec::new();
    for ci in 0..rule.clauses.len() {
        let cid = rule.clauses[ci].id.clone();
        let mut stages = Vec::new();
        let mut composite = Vec::new();
        let dump = {
            let Kind::Relational(plan) = &rule.clauses[ci].kind else { continue };
            for (si, st) in plan.stages.iter().enumerate() {
                if let StagePlan::Query { name, .. } = st {
                    if let Some((_, cls)) = shared.get(name.as_str()) {
                        if cls.iter().any(|(c, id)| c == check && *id == cid) {
                            if COMPOSITE.contains(&name.as_str()) {
                                composite.push((si, name.clone()));
                            } else {
                                stages.push((si, name.clone()));
                            }
                        }
                    }
                }
            }
            if stages.is_empty() && composite.is_empty() {
                continue;
            }
            let clause = resolved["clauses"].as_array().unwrap().iter().find(|c| c["id"] == cid.as_str()).unwrap();
            let dump = a3dump::plan(&clause["relational"], plan, &rule.inputs, &mut rule.names, &facts::GRAMMAR, &utils).unwrap();
            if dump.is_none() && plan.syntax.is_none() {
                continue;
            }
            dump
        };
        let tq = true_query(rule);
        let tq2 = true_query(rule);
        let Kind::Relational(plan) = &rule.clauses[ci].kind else { unreachable!() };
        let mut dump = dump;
        if let Some(g) = dump.as_mut() {
            g.gate = tq;
            if let Some(a) = g.ancestor.as_mut() {
                a.1 = tq2;
            }
        }
        // The stages the targets read, transitively, and the inputs outside the dump.
        let names: Vec<String> = plan.stages.iter().map(|s| match s {
            StagePlan::Query { name, .. } | StagePlan::Reach { name, .. } => name.to_ascii_lowercase(),
        }).collect();
        let mut need: BTreeSet<usize> = stages.iter().chain(&composite).map(|s| s.0).collect();
        let mut extra = BTreeSet::new();
        let mut work: Vec<usize> = need.iter().copied().collect();
        while let Some(i) = work.pop() {
            let reads = match &plan.stages[i] {
                StagePlan::Query { stmt, deferred, .. } => {
                    assert!(deferred.is_empty(), "{cid}: deferred stage");
                    stmt.reads.clone()
                }
                StagePlan::Reach { name, .. } => panic!("{cid}: reachability stage {name} needed"),
            };
            for r in reads {
                if let Some(j) = names.iter().position(|n| n.eq_ignore_ascii_case(&r)) {
                    if need.insert(j) {
                        work.push(j);
                    }
                } else if let Some(g) = &dump {
                    let member = g.members.iter().any(|m| plan.inputs[m.plan_input].0.eq_ignore_ascii_case(&r));
                    if !member {
                        let pi = plan.inputs.iter().position(|(n, _)| n.eq_ignore_ascii_case(&r)).unwrap_or_else(|| panic!("{cid}: read {r} is neither a stage nor an input"));
                        extra.insert(pi);
                    }
                }
            }
        }
        targets.push(Target { rule: ri, clause: ci, clause_id: cid, stages, composite, run: need.into_iter().collect(), extra: extra.into_iter().collect(), lowered: Vec::new(), dump });
    }
    targets
}

#[derive(Default, Clone)]
struct Stat {
    files: u64,
    rows: u64,
    bag_mismatch: u64,
    order_mismatch: u64,
    examples: Vec<String>,
    bag_examples: Vec<String>,
}

#[derive(Default)]
struct Out {
    stats: BTreeMap<(String, String), Stat>,
    /// (path index, bytes, nodes, parse, extract, views (core tables included), typedefs, core
    /// tables without string values, string values) ns.
    per_file: Vec<(u32, u32, u32, u64, u64, u64, u64, u64, u64)>,
    /// The last file's core-table and string-value ns (`views::evaluate`).
    last_core_ns: [u64; 2],
    /// Check-side time per (side, stage): `reference <stage>`, `view <stage>`, view-side inputs.
    stage_ns: BTreeMap<String, u64>,
    view_ns: Vec<u64>,
    view_rows: Vec<u64>,
    skipped_errors: u64,
    key_collisions: u64,
    nodes_mismatch: Vec<String>,
    extra_nodes: BTreeMap<String, u64>,
    failures: Vec<String>,
    annotations: u64,
    /// `--cg`: per view (`cg_def`, `cg_call`, `cg_import`): rows, files that differ, examples.
    cg: BTreeMap<&'static str, Stat>,
}

fn text_of(v: &V) -> String {
    match v {
        V::Text(s) => s.to_string(),
        V::Null => "NULL".into(),
        V::Int(i) => i.to_string(),
        o => format!("{o:?}"),
    }
}

/// Compares two multisets of rendered rows and records the outcome under `view`.
fn cg_compare(out: &mut Out, view: &'static str, path: &str, mut ours: Vec<String>, mut theirs: Vec<String>) {
    let st = out.cg.entry(view).or_default();
    st.files += 1;
    st.rows += ours.len() as u64;
    ours.sort();
    theirs.sort();
    if ours != theirs {
        st.bag_mismatch += 1;
        if st.bag_examples.len() < 5 {
            let only_view: Vec<&String> = ours.iter().filter(|x| theirs.binary_search(x).is_err()).take(3).collect();
            let only_ref: Vec<&String> = theirs.iter().filter(|x| ours.binary_search(x).is_err()).take(3).collect();
            st.bag_examples.push(format!("{path}: view {} rows, reference {} rows; only view {only_view:?}; only reference {only_ref:?}", ours.len(), theirs.len()));
        }
    }
}

/// `--cg`: SCHEMA § 3.3's views against their references. `cg_def` against native's
/// `Summary::extract` (no module path); `cg_import` against native's scope-table import bindings;
/// `cg_call.caller` and `cg_import.scope` against a walk of the facts that applies native's
/// context rule (`summary.rs::walk`: a named definition's body runs in its context, its other
/// children in the outer one; a class keeps the outer function as owner).
fn cg_check(f: &Facts, vdb: &Db<'_>, path: &str, out: &mut Out) {
    use ast_grep_core::AstGrep;
    use rerail_engine::code_rule_dsl::calls::summary::{ASYNC, CLASS, Summary};
    use rerail_engine::code_rule_dsl::language::GrammarDialect;
    use rerail_engine::code_rule_dsl::scope::{Import, Reading, ScopeTable};
    let Some(tape) = &f.tape else {
        out.failures.push(format!("{path}: cg check needs the parse"));
        return;
    };
    let grep = AstGrep::doc(tape.clone());
    let t = |n: &str| vdb.tables.get(n).unwrap().clone();
    // cg_def vs native defs.
    let summary = Summary::extract(&grep.root(), None, None, f.syntax_errors);
    let native: Vec<String> = summary
        .defs
        .iter()
        .map(|d| {
            let kind = if d.flags & CLASS != 0 { "class" } else if d.flags & ASYNC != 0 { "async_function" } else { "function" };
            format!("{}|{kind}|{}|{}", summary.name(d.name), d.start, d.end)
        })
        .collect();
    let defs = t("cg_def");
    let (k, ln, s, e) = (col(&defs, "kind"), col(&defs, "local_name"), col(&defs, "start"), col(&defs, "end"));
    let ours: Vec<String> = (0..defs.len()).map(|i| format!("{}|{}|{}|{}", text_of(&defs.value(i, ln)), text_of(&defs.value(i, k)), text_of(&defs.value(i, s)), text_of(&defs.value(i, e)))).collect();
    cg_compare(out, "cg_def", path, ours, native);
    // cg_refusal (SCHEMA § 3.3): any row makes a consumer refuse the content; recorded as a
    // difference so the run shows where it happens.
    let refusal = t("cg_refusal");
    let rows: Vec<String> = (0..refusal.len()).map(|i| format!("{}|{}", text_of(&refusal.value(i, 0)), text_of(&refusal.value(i, 1)))).collect();
    cg_compare(out, "cg_refusal", path, rows, Vec::new());
    // Span keys (for S1's declared keys on matcher-input rows): nodes of one kind sharing a span,
    // over every node and over named nodes; recorded as rows = collisions.
    for (label, named_only) in [("span.same_kind_all_nodes", false), ("span.same_kind_named_nodes", true)] {
        let mut seen = rustc_hash::FxHashSet::default();
        let mut hits = Vec::new();
        for x in 0..f.len() as u32 {
            if named_only && !f.named(x) {
                continue;
            }
            if !seen.insert((f.kind[x as usize], f.start[x as usize], f.end[x as usize])) {
                hits.push(format!("{}:{}:{}", facts::name(f.kind[x as usize]), f.start[x as usize], f.end[x as usize]));
            }
        }
        cg_compare(out, label, path, hits, Vec::new());
    }
    // Native's context rule over the facts: (owner function, nearest scope) per node.
    let n = f.len();
    let fname = |c: u32| facts::field_name(f.field[c as usize]);
    let mut ctx: Vec<(u32, u32)> = vec![(NONE, NONE); n];
    for x in 1..n as u32 {
        let p = f.parent[x as usize];
        let pk = facts::name(f.kind[p as usize]);
        let mut c = ctx[p as usize];
        if &*pk == "function_definition" || &*pk == "class_definition" {
            let first = |field: &str| f.children(p).iter().copied().find(|&ch| fname(ch) == field);
            if first("name").is_some() && first("body") == Some(x) {
                c = (if &*pk == "function_definition" { p } else { c.0 }, p);
            }
        }
        ctx[x as usize] = c;
    }
    let id = |x: u32| if x == NONE { "NULL".to_string() } else { x.to_string() };
    let call_kind = facts::lookup("call");
    let mut reference = Vec::new();
    for &c in call_kind.map_or(&[][..], |k| f.of_kind(k)) {
        if let Some(func) = f.children(c).iter().copied().find(|&ch| fname(ch) == "function") {
            reference.push(format!("{c}|{}|{}|{}|{}", f.start[c as usize], f.end[c as usize], f.text(func), id(ctx[c as usize].0)));
        }
    }
    let calls = t("cg_call");
    let cols = ["node", "start", "end", "callee_text", "caller"].map(|c| col(&calls, c));
    let ours: Vec<String> = (0..calls.len()).map(|i| cols.iter().map(|&c| text_of(&calls.value(i, c))).collect::<Vec<_>>().join("|")).collect();
    cg_compare(out, "cg_call", path, ours, reference);
    // cg_import vs native import bindings, rendered alike: kind | bound name | import | submodule.
    let imports = t("cg_import");
    let ic = ["node", "module_text", "name", "alias", "level", "scope"].map(|c| col(&imports, c));
    let mut ours = Vec::new();
    let mut scopes_ours = Vec::new();
    let mut scopes_ref = Vec::new();
    for i in 0..imports.len() {
        let v: Vec<V> = ic.iter().map(|&c| imports.value(i, c)).collect();
        let node = int(&v[0]);
        scopes_ours.push(format!("{node}|{}", text_of(&v[5])));
        scopes_ref.push(format!("{node}|{}", id(ctx[node as usize].1)));
        let (module, name, alias, level) = (text_of(&v[1]), &v[2], &v[3], int(&v[4]));
        let stmt = facts::name(f.kind[node as usize]);
        let row = if &*stmt == "import_statement" {
            match alias {
                V::Text(a) => format!("Module|{a}|Qualified({module:?})|None"),
                _ if module.contains('.') => {
                    let head = module.split('.').next().unwrap();
                    format!("Submodule|{head}|Qualified({head:?})|Some({module:?})")
                }
                _ => format!("Module|{module}|Qualified({module:?})|None"),
            }
        } else {
            let name = text_of(name);
            if name == "*" {
                continue;
            }
            let bound = match alias {
                V::Text(a) => a.to_string(),
                _ => name.clone(),
            };
            let kind = if &*stmt == "future_import_statement" { "Future" } else { "Member" };
            let import = if level > 0 {
                let rest = if module.is_empty() { name.clone() } else { format!("{module}.{name}") };
                format!("Relative {{ level: {level}, rest: {rest:?} }}")
            } else {
                format!("Qualified({:?})", format!("{module}.{name}"))
            };
            format!("{kind}|{bound}|{import}|None")
        };
        ours.push(row);
    }
    cg_compare(out, "cg_import.scope", path, scopes_ours, scopes_ref);
    match ScopeTable::build_in(grep.root(), GrammarDialect::Python, usize::MAX, &|| false, Reading::default(), None) {
        Ok(table) => {
            let mut theirs = Vec::new();
            for scope in &table.scopes {
                for (bound, bindings) in &scope.bindings {
                    for b in bindings {
                        let Some(kind) = b.import_kind else { continue };
                        let import = match &b.import {
                            Some(Import::Qualified(q)) => format!("Qualified({:?})", q.as_ref()),
                            Some(Import::Relative { level, rest }) => format!("Relative {{ level: {level}, rest: {:?} }}", rest.as_ref()),
                            other => format!("{other:?}"),
                        };
                        theirs.push(format!("{kind:?}|{bound}|{import}|{:?}", b.submodule.as_deref()));
                    }
                }
            }
            cg_compare(out, "cg_import", path, ours, theirs);
        }
        Err(e) => out.failures.push(format!("{path}: scope table: {e:?}")),
    }
}

fn col<'t>(t: &'t Table, name: &str) -> usize {
    t.schema.cols.iter().position(|c| c == name).unwrap_or_else(|| panic!("column {name} in {:?}", t.schema.cols))
}

fn int(v: &V) -> u32 {
    match v {
        V::Int(i) => *i as u32,
        V::Null => NONE,
        o => panic!("id {o:?}"),
    }
}

struct Render<'a> {
    f: &'a Facts,
    path: Rc<str>,
    dump: bool,
    nodes: Rc<Table>,
    rank: Vec<u32>,
}

impl Render<'_> {
    fn id(&self, x: u32) -> V {
        if x == NONE {
            V::Null
        } else if self.dump {
            V::text(&format!("{}:{}:{}", facts::name(self.f.kind[x as usize]), self.f.start[x as usize], self.f.end[x as usize]))
        } else {
            V::Int(x as i64)
        }
    }
    fn text(&self, x: u32) -> V {
        if x == NONE { V::Null } else { V::text(self.f.text(x)) }
    }
    fn kind(&self, x: u32) -> V {
        if x == NONE { V::Null } else { V::Text(facts::name(self.f.kind[x as usize])) }
    }
    fn row(&self, x: u32) -> Vec<V> {
        let r = self.rank[x as usize];
        assert!(r != NONE, "node {x} not in nodes");
        self.nodes.row(r as usize)
    }
    fn rank(&self, x: u32) -> u32 {
        if x == NONE { 0 } else { self.rank[x as usize] }
    }
}

fn tri_and(a: &V, b: &V) -> V {
    match (a, b) {
        (V::Int(0), _) | (_, V::Int(0)) => V::Int(0),
        (V::Null, _) | (_, V::Null) => V::Null,
        _ => V::Int(1),
    }
}

/// The view's rows rendered as the stage's, with their sort keys.
fn render(stage: &str, vdb: &Db<'_>, r: &Render<'_>) -> Vec<(Vec<u32>, Vec<V>)> {
    let t = |n: &str| vdb.tables.get(n).unwrap().clone();
    let mut out = Vec::new();
    let fname: BTreeMap<u32, u32> = {
        let fv = t("py_function");
        (0..fv.len()).map(|i| (int(&fv.value(i, 0)), int(&fv.value(i, 1)))).collect()
    };
    let func_row = |x: u32| {
        let mut row = r.row(x);
        row.push(r.text(fname[&x]));
        row
    };
    match stage {
        "ownership" | "ownership_named" => {
            let v = t(if stage == "ownership" { "py_owner" } else { "py_owner_named" });
            for i in 0..v.len() {
                let x = v.row(i);
                let id = int(&x[0]);
                out.push((vec![r.rank(id)], vec![V::Text(r.path.clone()), r.id(id), r.id(int(&x[1])), r.id(int(&x[2])), r.id(int(&x[3]))]));
            }
        }
        "functions" | "classes" => {
            let v = t(if stage == "functions" { "py_function" } else { "py_class" });
            for i in 0..v.len() {
                let x = v.row(i);
                let id = int(&x[0]);
                let mut row = r.row(id);
                row.push(r.text(int(&x[1])));
                out.push((vec![r.rank(id)], row));
            }
        }
        "methods" => {
            let v = t("py_method");
            let cv = t("py_class");
            let cname: BTreeMap<u32, u32> = (0..cv.len()).map(|i| (int(&cv.value(i, 0)), int(&cv.value(i, 1)))).collect();
            for i in 0..v.len() {
                let (fid, cid) = (int(&v.value(i, 0)), int(&v.value(i, 1)));
                let mut row = func_row(fid);
                row.push(r.id(cid));
                row.push(r.text(cname[&cid]));
                out.push((vec![r.rank(fid), r.rank(cid)], row));
            }
        }
        "attributes" => {
            let v = t("py_attribute");
            for i in 0..v.len() {
                let x = v.row(i);
                let (id, attr, obj) = (int(&x[0]), int(&x[1]), int(&x[2]));
                let mut row = r.row(id);
                row.extend([r.text(attr), r.id(obj), r.text(obj)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "calls" => {
            let v = t("py_call");
            for i in 0..v.len() {
                let x = v.row(i);
                let (id, func, callee, recv) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let mut row = r.row(id);
                row.extend([r.id(func), r.kind(func), r.text(func), r.text(callee), r.id(recv)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "parameters" => {
            let v = t("py_parameter");
            for i in 0..v.len() {
                let x = v.row(i);
                let (fid, p, name, ann) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let row = vec![V::Text(r.path.clone()), r.id(fid), r.id(p), r.kind(p), r.text(p), r.text(name), r.id(ann), x[4].clone()];
                out.push((vec![r.rank(fid), r.rank(p)], row));
            }
        }
        "assignments" => {
            let v = t("py_assignment");
            for i in 0..v.len() {
                let x = v.row(i);
                let (id, tg, val, fid) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let mut row = r.row(id);
                row.extend([r.id(tg), r.kind(tg), r.text(tg), r.id(val), r.kind(val), r.text(val), r.id(fid)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "decorators" => {
            let v = t("py_decorator");
            for i in 0..v.len() {
                let x = v.row(i);
                let (fid, d, child, name) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                out.push((vec![r.rank(fid), r.rank(d), r.rank(child)], vec![V::Text(r.path.clone()), r.id(fid), r.id(d), r.text(name)]));
            }
        }
        "import_identifiers" => {
            let v = t("py_import_identifier");
            for i in 0..v.len() {
                let (id, imp) = (int(&v.value(i, 0)), int(&v.value(i, 1)));
                out.push((vec![r.rank(imp), r.rank(id)], vec![V::Text(r.path.clone()), r.id(id)]));
            }
        }
        "contexts" => {
            let v = t("py_context");
            let c = |n: &str| col(&v, n);
            let (cn, pl, pp, ch, ph, ma) = (c("class_name"), c("planning"), c("preprocess"), c("ctx_helper"), c("provider_helper_name"), c("materializer"));
            for i in 0..v.len() {
                let x = v.row(i);
                let fid = int(&x[0]);
                let mut row = func_row(fid);
                let provider = tri_and(&V::bool(&*r.path == PROVIDER_PATH), &x[ph]);
                row.extend([x[cn].clone(), x[pl].clone(), x[pp].clone(), x[ch].clone(), provider, x[ma].clone()]);
                out.push((vec![r.rank(fid)], row));
            }
        }
        s => panic!("stage {s}"),
    }
    out
}

fn key(row: &[V]) -> String {
    format!("{row:?}")
}

/// `input_max_nodes`: above it, a § 2.4 composition takes its matcher inputs from the leaf
/// executor on both sides (the reference evaluator of the lowered inputs is naive); such files
/// are counted as `inputs unchecked` (the inputs' own differential is `facts-a pattern-check`).
fn check_file(rules: &[Rule], targets: &[Target], views_c: &Views, f: &Facts, path: &str, source: &str, out: &mut Out, check: bool, cg: bool, input_max_nodes: usize) -> u64 {
    let mut vns = Vec::new();
    let t0 = Instant::now();
    let vdb = match views::evaluate(views_c, f, &mut vns, &mut out.last_core_ns) {
        Ok(db) => db,
        Err(e) => {
            out.failures.push(format!("{path}: {e}"));
            return 0;
        }
    };
    let views_ns = t0.elapsed().as_nanos() as u64;
    if out.view_ns.is_empty() {
        out.view_ns = vec![0; vns.len()];
        out.view_rows = vec![0; vns.len()];
    }
    for (i, (name, _)) in views_c.stmts.iter().enumerate() {
        out.view_ns[i] += vns[i];
        out.view_rows[i] += vdb.tables[name].len() as u64;
    }
    if cg {
        cg_check(f, &vdb, path, out);
    }
    if !check {
        return views_ns;
    }
    if f.syntax_errors {
        out.skipped_errors += 1;
        return views_ns;
    }
    // N, and `kind:start:end` uniqueness over it.
    let py_node = vdb.tables["py_node"].clone();
    {
        let mut seen = rustc_hash::FxHashSet::default();
        for i in 0..py_node.len() {
            let id = int(&py_node.value(i, 0)) as usize;
            if !seen.insert((f.kind[id], f.start[id], f.end[id])) {
                out.key_collisions += 1;
            }
        }
    }
    let pathrc: Rc<str> = Rc::from(path);
    let mut core = None;
    let mut input_cache = BTreeMap::new();
    for tg in targets {
        let rule = &rules[tg.rule];
        let Kind::Relational(plan) = &rule.clauses[tg.clause].kind else { unreachable!() };
        let mut db = Db::new(source, path);
        let dump = tg.dump.is_some();
        let ctx = eval::FileCtx { f, path, source };
        // The tables the stages start from (dump members or syntax-spec tables, matcher inputs).
        let mut base: Vec<(String, Rc<Table>)> = Vec::new();
        if let Some(g) = &tg.dump {
            let Some(tables) = a3dump::rows(rule, g, plan, &ctx) else {
                out.failures.push(format!("{path}: {}: dump refused", tg.clause_id));
                continue;
            };
            for (m, t) in g.members.iter().zip(tables) {
                base.push((plan.inputs[m.plan_input].0.to_ascii_lowercase(), Rc::new(t)));
            }
        } else if let Some(spec) = &plan.syntax {
            for (n, t) in spec.tables(f, path) {
                base.push((n.to_ascii_lowercase(), Rc::new(t)));
            }
        }
        // Matcher inputs outside the dump, gate-free, by the leaf executor (the reference).
        let memo = crate::ast::FileMemo::default();
        let text: Rc<str> = Rc::from(source);
        let mut refused = None;
        let mut leaf_inputs = Vec::new();
        for &pi in &tg.extra {
            let (name, ri) = &plan.inputs[pi];
            let r = rule.inputs[*ri].rows(&ctx, &plan.input_schemas[pi], &memo, &text);
            if let Some(u) = r.unsupported {
                refused = Some(format!("{path}: {} input {name}: {u}", tg.clause_id));
                break;
            }
            leaf_inputs.push((name.to_ascii_lowercase(), r.table));
        }
        if let Some(u) = refused {
            out.failures.push(u);
            continue;
        }
        for (n, t) in base.iter().chain(&leaf_inputs) {
            db.tables.insert(n.clone(), t.clone());
        }
        for &si in &tg.run {
            let StagePlan::Query { name, stmt, .. } = &plan.stages[si] else { unreachable!() };
            let t0 = Instant::now();
            let result = engine::execute(&db, stmt);
            *out.stage_ns.entry(format!("reference {name}")).or_default() += t0.elapsed().as_nanos() as u64;
            match result {
                Ok(t) => {
                    let t = views::store(t, stmt.schema());
                    db.tables.insert(name.to_ascii_lowercase(), Rc::new(t));
                }
                Err(e) => {
                    out.failures.push(format!("{path}: {} stage {name}: {e}", tg.clause_id));
                    break;
                }
            }
        }
        let Some(nodes) = db.tables.get("nodes").cloned() else {
            out.failures.push(format!("{path}: {}: no nodes", tg.clause_id));
            continue;
        };
        // Ranks of N's nodes in the clause's `nodes`.
        let idc = col(&nodes, "id");
        let mut rank = vec![NONE; f.len()];
        let mut by_key: rustc_hash::FxHashMap<String, u32> = Default::default();
        for i in 0..nodes.len() {
            match nodes.value(i, idc) {
                V::Int(x) => rank[x as usize] = i as u32,
                V::Text(s) => {
                    by_key.insert(s.to_string(), i as u32);
                }
                o => panic!("nodes id {o:?}"),
            }
        }
        let r0 = Render { f, path: pathrc.clone(), dump, nodes: nodes.clone(), rank: Vec::new() };
        // The clause's gate-true node set is N (`py_node`) or all named nodes (`py_named`).
        let py_named = vdb.tables["py_named"].clone();
        let all_named = nodes.len() != py_node.len();
        let set = if all_named { &py_named } else { &py_node };
        if dump {
            for i in 0..set.len() {
                let id = int(&set.value(i, 0));
                if let V::Text(k) = r0.id(id) {
                    if let Some(&i) = by_key.get(&*k) {
                        rank[id as usize] = i;
                    }
                }
            }
        }
        let missing = (0..set.len()).filter(|&i| rank[int(&set.value(i, 0)) as usize] == NONE).count();
        if missing > 0 || nodes.len() != set.len() {
            out.nodes_mismatch.push(format!("{path}: {}: nodes {} vs N {} / named {} ({missing} missing)", tg.clause_id, nodes.len(), py_node.len(), py_named.len()));
            continue;
        }
        if all_named {
            *out.extra_nodes.entry(tg.clause_id.clone()).or_default() += (nodes.len() - py_node.len()) as u64;
        }
        let r = Render { rank, ..r0 };
        for (si, sname) in &tg.stages {
            let StagePlan::Query { stmt, .. } = &plan.stages[*si] else { unreachable!() };
            let Some(reference) = db.tables.get(&sname.to_ascii_lowercase()).cloned() else { continue };
            let view_stage = if all_named && sname == "ownership" { "ownership_named" } else { sname.as_str() };
            let mut rows = render(view_stage, &vdb, &r);
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            let expected: Vec<Box<[V]>> = rows.into_iter().map(|(_, row)| row.into_boxed_slice()).collect();
            let st = out.stats.entry((sname.clone(), tg.clause_id.clone())).or_default();
            st.files += 1;
            st.rows += reference.len() as u64;
            // A rendered row of the wrong width would be cut (or misaligned) by the columnar store.
            let width = stmt.schema().cols.len();
            if let Some(x) = expected.iter().find(|x| x.len() != width) {
                st.bag_mismatch += 1;
                if st.bag_examples.len() < 3 {
                    st.bag_examples.push(format!("{path}: view row width {} vs stage {width}: {}", x.len(), key(x)));
                }
                continue;
            }
            let expected = views::store(Table::from_rows(stmt.schema().clone(), expected), stmt.schema());
            let a: Vec<String> = (0..expected.len()).map(|i| key(&expected.row(i))).collect();
            let b: Vec<String> = (0..reference.len()).map(|i| key(&reference.row(i))).collect();
            compare_rows(st, path, a, b);
        }
        if tg.composite.is_empty() {
            continue;
        }
        // § 2.4: the same stages over the views rendered in place of the stages they stand for,
        // and the matcher inputs from the view side (`view_inputs`).
        let mut vdb2 = Db::new(source, path);
        let t0 = Instant::now();
        let inputs = if f.len() <= input_max_nodes {
            view_inputs(plan, tg, f, path, &mut core, &mut input_cache)
        } else {
            out.stats.entry(("inputs unchecked (file above --input-max-nodes)".into(), tg.clause_id.clone())).or_default().files += 1;
            Ok(leaf_inputs.clone())
        };
        *out.stage_ns.entry("view-side inputs (reference evaluator)".into()).or_default() += t0.elapsed().as_nanos() as u64;
        let inputs = match inputs {
            Ok(t) => t,
            Err(e) => {
                out.failures.push(format!("{path}: {}: view-side inputs: {e}", tg.clause_id));
                continue;
            }
        };
        // The matcher inputs themselves: lowered algebra against the leaf executor.
        for (n, t) in &inputs {
            let Some((_, l)) = leaf_inputs.iter().find(|(m, _)| m == n) else { continue };
            let st = out.stats.entry((format!("input {n}"), tg.clause_id.clone())).or_default();
            st.files += 1;
            st.rows += l.len() as u64;
            compare_rows(st, path, (0..t.len()).map(|i| key(&t.row(i))).collect(), (0..l.len()).map(|i| key(&l.row(i))).collect());
        }
        for (n, t) in base.iter().chain(&inputs) {
            vdb2.tables.insert(n.clone(), t.clone());
        }
        let mut failed = false;
        for &si in &tg.run {
            let StagePlan::Query { name, stmt, .. } = &plan.stages[si] else { unreachable!() };
            let t0 = Instant::now();
            let t = if let Some((_, sname)) = tg.stages.iter().find(|(i, _)| *i == si) {
                let view_stage = if all_named && sname == "ownership" { "ownership_named" } else { sname.as_str() };
                let mut rows = render(view_stage, &vdb, &r);
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                let rows: Vec<Box<[V]>> = rows.into_iter().map(|(_, row)| row.into_boxed_slice()).collect();
                views::store(Table::from_rows(stmt.schema().clone(), rows), stmt.schema())
            } else {
                match engine::execute(&vdb2, stmt) {
                    Ok(t) => views::store(t, stmt.schema()),
                    Err(e) => {
                        out.failures.push(format!("{path}: {} view-side stage {name}: {e}", tg.clause_id));
                        failed = true;
                        break;
                    }
                }
            };
            *out.stage_ns.entry(format!("view {name}")).or_default() += t0.elapsed().as_nanos() as u64;
            vdb2.tables.insert(name.to_ascii_lowercase(), Rc::new(t));
        }
        if failed {
            continue;
        }
        for (_, cname) in &tg.composite {
            let lname = cname.to_ascii_lowercase();
            let (Some(reference), Some(view)) = (db.tables.get(&lname).cloned(), vdb2.tables.get(&lname).cloned()) else { continue };
            let st = out.stats.entry((cname.clone(), tg.clause_id.clone())).or_default();
            st.files += 1;
            st.rows += reference.len() as u64;
            let a: Vec<String> = (0..view.len()).map(|i| key(&view.row(i))).collect();
            let b: Vec<String> = (0..reference.len()).map(|i| key(&reference.row(i))).collect();
            compare_rows(st, path, a, b);
        }
    }
    views_ns
}

/// View rows `a` against stage rows `b`: as multisets, then in order.
fn compare_rows(st: &mut Stat, path: &str, mut a: Vec<String>, mut b: Vec<String>) {
    let ordered = a == b;
    a.sort();
    b.sort();
    if a != b {
        st.bag_mismatch += 1;
        if st.bag_examples.len() < 3 {
            let only_view: Vec<&String> = a.iter().filter(|x| b.binary_search(x).is_err()).take(2).collect();
            let only_ref: Vec<&String> = b.iter().filter(|x| a.binary_search(x).is_err()).take(2).collect();
            st.bag_examples.push(format!("{path}: bag: view {} rows, stage {} rows; only view {only_view:?}; only stage {only_ref:?}", a.len(), b.len()));
        }
    } else if !ordered {
        st.order_mismatch += 1;
        if st.examples.len() < 3 {
            st.examples.push(format!("{path}: order differs ({} rows)", a.len()));
        }
    }
}

/// The view side's matcher inputs for a § 2.4 composition (`tg.extra`, gate-free): each input's
/// algebra from the pattern front end (`tg.lowered`), evaluated by `rerail_facts::reference` over
/// the content's core relations (`views::core_rows`, `string_value` included; built once per
/// file in `core`). Identical algebra is evaluated once per file (`cache`). A run-time refusal of
/// the algebra (`refuse_if`) must equal the leaf executor's, which `check_file` already surfaced;
/// here it is an error.
fn view_inputs(
    plan: &crate::rel::RelPlan,
    tg: &Target,
    f: &Facts,
    path: &str,
    core: &mut Option<BTreeMap<rerail_facts::ir::CoreRel, Vec<rerail_facts::reference::Row>>>,
    cache: &mut BTreeMap<String, Vec<rerail_facts::reference::Row>>,
) -> Result<Vec<(String, Rc<Table>)>, String> {
    use rerail_facts::ir::FileSource;
    let core = core.get_or_insert_with(|| views::core_rows(f, path));
    let scan = |s: &FileSource| -> Result<Vec<rerail_facts::reference::Row>, String> {
        match s {
            FileSource::Core(c) => Ok(core[c].clone()),
            o => Err(format!("{o:?} in a lowered input")),
        }
    };
    let ev = rerail_facts::reference::Evaluator::with_content(&scan, path, f.source.as_str());
    let mut out = Vec::new();
    for (pi, lowered) in &tg.lowered {
        let (name, _) = &plan.inputs[*pi];
        let lowered = lowered.as_ref().map_err(|e| format!("input {name} not lowered: {e}"))?;
        let key = format!("{lowered:?}");
        if !cache.contains_key(&key) {
            for (r, msg) in &lowered.refuse_if {
                if !ev.rel(r, &[])?.is_empty() {
                    return Err(format!("input {name}: lowered refusal {msg}"));
                }
            }
            cache.insert(key.clone(), ev.rel(&lowered.rows, &[])?);
        }
        let rows = cache[&key].iter().map(|r| r.clone().into_boxed_slice()).collect();
        out.push((name.to_ascii_lowercase(), Rc::new(Table::from_rows(plan.input_schemas[*pi].clone(), rows))));
    }
    Ok(out)
}

pub fn run(args: &[String]) {
    let root = arg(args, "--root").expect("--root");
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    let only: Option<String> = arg(args, "--only");
    let check = !args.iter().any(|a| a == "--no-check");
    // `--typedefs`: also time `string_type_definition` per file (SCHEMA § 1.3).
    let typedefs = args.iter().any(|a| a == "--typedefs");
    // `--cg`: SCHEMA § 3.3's call-graph views against native (`cg_check`).
    let cg = args.iter().any(|a| a == "--cg");
    // `--input-max-nodes N`: the § 2.4 inputs' reference-evaluator bound (`check_file`).
    let input_max_nodes: usize = arg(args, "--input-max-nodes").map_or(usize::MAX, |n| n.parse().unwrap());
    std::fs::create_dir_all(&out_dir).unwrap();
    let root = Path::new(&root);
    let mut paths = walk(root);
    if let Some(o) = &only {
        paths.retain(|p| p.contains(o.as_str()));
    }
    if let Some(l) = limit {
        paths.truncate(l);
    }
    // `--shard k/n`: every n-th file from the k-th (runs stay under the 15-minute limit).
    if let Some(s) = arg(args, "--shard") {
        let (k, n) = s.split_once('/').map(|(k, n)| (k.parse::<usize>().unwrap(), n.parse::<usize>().unwrap())).expect("--shard k/n");
        paths = paths.into_iter().enumerate().filter(|(i, _)| i % n == k).map(|(_, p)| p).collect();
    }
    let shared = shared_texts(&policies);
    // Workers take files largest first, so the slowest file does not start last; the set checked and
    // every per-file result are unchanged (`per_file` is keyed by the index into `paths`).
    let mut order: Vec<usize> = (0..paths.len()).collect();
    order.sort_by_cached_key(|&i| std::cmp::Reverse(std::fs::metadata(root.join(&paths[i])).map_or(0, |m| m.len())));
    let t_all = Instant::now();
    let next = AtomicUsize::new(0);
    let outs: Vec<Out> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                let (paths, order, next, policies, shared) = (&paths, &order, &next, &policies, &shared);
                s.spawn(move || {
                    let (rules, targets) = if check { compile(policies, shared) } else { (Vec::new(), Vec::new()) };
                    let vc = Views::compile().unwrap();
                    let mut out = Out::default();
                    loop {
                        let Some(&i) = order.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                        let path = &paths[i];
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let source = Arc::new(src);
                        let mut ns = facts::BuildNs::default();
                        let Ok(facts::Built::Facts(f)) = facts::build_timed(path, source.clone(), &mut ns) else { continue };
                        let vns = check_file(&rules, &targets, &vc, &f, path, &source, &mut out, check, cg, input_max_nodes);
                        let mut tns = 0;
                        if typedefs {
                            let t = Instant::now();
                            match f.typedefs() {
                                Ok(d) => out.annotations += d.len() as u64,
                                Err(e) => out.failures.push(format!("{path}: typedefs: {e}")),
                            }
                            tns = t.elapsed().as_nanos() as u64;
                        }
                        let core = out.last_core_ns;
                        out.per_file.push((i as u32, source.len() as u32, f.len() as u32, ns.parse, ns.extract, vns, tns, core[0], core[1]));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let wall = t_all.elapsed().as_secs_f64();
    let vc = Views::compile().unwrap();
    let mut stats: BTreeMap<(String, String), Stat> = BTreeMap::new();
    let mut per_file = Vec::new();
    let mut view_ns = vec![0u64; vc.stmts.len()];
    let mut view_rows = vec![0u64; vc.stmts.len()];
    let (mut skipped, mut collisions, mut annotations) = (0, 0, 0u64);
    let mut cg_stats: BTreeMap<&'static str, Stat> = BTreeMap::new();
    let mut stage_ns: BTreeMap<String, u64> = BTreeMap::new();
    let (mut nodes_mismatch, mut failures) = (Vec::new(), Vec::new());
    let mut extra_nodes: BTreeMap<String, u64> = BTreeMap::new();
    for o in outs {
        for (k, v) in o.stats {
            let e = stats.entry(k).or_default();
            e.files += v.files;
            e.rows += v.rows;
            e.bag_mismatch += v.bag_mismatch;
            e.order_mismatch += v.order_mismatch;
            e.examples.extend(v.examples);
            e.bag_examples.extend(v.bag_examples);
        }
        per_file.extend(o.per_file);
        for (i, x) in o.view_ns.iter().enumerate() {
            view_ns[i] += x;
            view_rows[i] += o.view_rows[i];
        }
        skipped += o.skipped_errors;
        annotations += o.annotations;
        collisions += o.key_collisions;
        nodes_mismatch.extend(o.nodes_mismatch);
        for (k, v) in o.extra_nodes {
            *extra_nodes.entry(k).or_default() += v;
        }
        failures.extend(o.failures);
        for (k, v) in o.stage_ns {
            *stage_ns.entry(k).or_default() += v;
        }
        for (k, v) in o.cg {
            let e = cg_stats.entry(k).or_default();
            e.files += v.files;
            e.rows += v.rows;
            e.bag_mismatch += v.bag_mismatch;
            e.bag_examples.extend(v.bag_examples);
        }
    }
    per_file.sort();
    let mut tsv = String::from("path\tbytes\tnodes\tparse_ns\textract_ns\tviews_ns\ttypedefs_ns\tcore_ns\tstring_value_ns\n");
    for p in &per_file {
        tsv.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", paths[p.0 as usize], p.1, p.2, p.3, p.4, p.5, p.6, p.7, p.8));
    }
    std::fs::write(Path::new(&out_dir).join("per-file.tsv"), tsv).unwrap();
    let stage_json: Vec<Json> = stats.iter().map(|((s, c), v)| json!({"stage": s, "clause": c, "files": v.files, "rows": v.rows, "bag_mismatch": v.bag_mismatch, "order_mismatch": v.order_mismatch, "bag_examples": v.bag_examples, "examples": v.examples})).collect();
    let total_bag: u64 = stats.values().map(|v| v.bag_mismatch).sum();
    let total_order: u64 = stats.values().map(|v| v.order_mismatch).sum();
    let res = json!({
        "files": per_file.len(),
        "wall_s": wall,
        "check": check,
        "shared_texts": shared.iter().map(|(k, (_, cls))| (k.clone(), json!(cls))).collect::<BTreeMap<_, _>>(),
        "skipped_syntax_errors": skipped,
        "kind_start_end_collisions": collisions,
        "nodes_mismatch": nodes_mismatch,
        "extra_nodes_beyond_n": extra_nodes,
        "failures": failures,
        "bag_mismatch_total": total_bag,
        "order_mismatch_total": total_order,
        "stages": stage_json,
        "views": vc.stmts.iter().enumerate().map(|(i, (n, _))| json!({"view": n, "ns": view_ns[i], "rows": view_rows[i]})).collect::<Vec<_>>(),
        "parse_ns": summary(per_file.iter().map(|p| p.3).collect()),
        "extract_ns": summary(per_file.iter().map(|p| p.4).collect()),
        "parse_extract_ns": summary(per_file.iter().map(|p| p.3 + p.4).collect()),
        "views_ns": summary(per_file.iter().map(|p| p.5).collect()),
        "typedefs_ns": summary(per_file.iter().map(|p| p.6).collect()),
        "core_tables_ns": summary(per_file.iter().map(|p| p.7).collect()),
        "string_value_ns": summary(per_file.iter().map(|p| p.8).collect()),
        "annotations": annotations,
        "check_stage_s": stage_ns.iter().map(|(k, v)| (k.clone(), *v as f64 / 1e9)).collect::<BTreeMap<_, _>>(),
        "cg": cg_stats.iter().map(|(k, v)| (k.to_string(), json!({"files": v.files, "rows": v.rows, "files_differing": v.bag_mismatch, "examples": v.bag_examples}))).collect::<BTreeMap<_, _>>(),
    });
    std::fs::write(Path::new(&out_dir).join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&json!({"files": res["files"], "wall_s": wall, "bag_mismatch_total": total_bag, "order_mismatch_total": total_order, "failures": res["failures"].as_array().map(|a| a.len()), "nodes_mismatch": res["nodes_mismatch"].as_array().map(|a| a.len()), "collisions": collisions, "skipped": skipped, "views_ns": res["views_ns"], "parse_extract_ns": res["parse_extract_ns"], "cg": cg_stats.iter().map(|(k, v)| (k.to_string(), json!([v.rows, v.bag_mismatch]))).collect::<BTreeMap<_, _>>()})).unwrap());
}
