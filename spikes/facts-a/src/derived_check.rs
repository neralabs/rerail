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

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Target {
    rule: usize,
    clause: usize,
    clause_id: String,
    /// (stage index, stage name)
    stages: Vec<(usize, String)>,
    /// Stage indices to run, in order.
    run: Vec<usize>,
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
                if !TARGETS.iter().any(|(s, _)| *s == name) {
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

/// Batch 6 compiled as the suite's production event, gates forced `true` in target clauses.
fn compile(policies: &str, shared: &BTreeMap<String, (String, Vec<(String, String)>)>) -> (Vec<Rule>, Vec<Target>) {
    let event = json!({"production_scope": true});
    let mut rules = Vec::new();
    let mut targets = Vec::new();
    for (ri, check) in BATCH6.iter().enumerate() {
        let doc = policy_doc(policies, check);
        let (config, scope) = call_config(&doc, &event);
        let mut rule = eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR).unwrap();
        for ci in 0..rule.clauses.len() {
            let cid = rule.clauses[ci].id.clone();
            let mut stages = Vec::new();
            {
                let Kind::Relational(plan) = &rule.clauses[ci].kind else { continue };
                for (si, st) in plan.stages.iter().enumerate() {
                    if let StagePlan::Query { name, .. } = st {
                        if let Some((_, cls)) = shared.get(name.as_str()) {
                            if cls.iter().any(|(c, id)| c == check && *id == cid) {
                                stages.push((si, name.clone()));
                            }
                        }
                    }
                }
                if stages.is_empty() || (plan.dump.is_none() && plan.syntax.is_none()) {
                    continue;
                }
            }
            let tq = true_query(&mut rule);
            let tq2 = true_query(&mut rule);
            let Kind::Relational(plan) = &mut rule.clauses[ci].kind else { unreachable!() };
            if let Some(g) = plan.dump.as_mut() {
                g.gate = tq;
                if let Some(a) = g.ancestor.as_mut() {
                    a.1 = tq2;
                }
            }
            // The stages the targets read, transitively.
            let names: Vec<String> = plan.stages.iter().map(|s| match s {
                StagePlan::Query { name, .. } | StagePlan::Reach { name, .. } => name.to_ascii_lowercase(),
            }).collect();
            let mut need: BTreeSet<usize> = stages.iter().map(|s| s.0).collect();
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
                    } else if plan.dump.is_some() {
                        // A non-stage read must be a dump member.
                        let ok = plan.dump.as_ref().unwrap().members.iter().any(|m| plan.inputs[m.plan_input].0.eq_ignore_ascii_case(&r));
                        assert!(ok, "{cid}: target reads input {r} outside the dump");
                    }
                }
            }
            targets.push(Target { rule: ri, clause: ci, clause_id: cid, stages, run: need.into_iter().collect() });
        }
        rules.push(rule);
    }
    (rules, targets)
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
    per_file: Vec<(u32, u32, u32, u64, u64, u64, u64)>,
    view_ns: Vec<u64>,
    view_rows: Vec<u64>,
    skipped_errors: u64,
    key_collisions: u64,
    nodes_mismatch: Vec<String>,
    extra_nodes: BTreeMap<String, u64>,
    failures: Vec<String>,
    annotations: u64,
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
        self.nodes.rows[r as usize].to_vec()
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
        fv.rows.iter().map(|x| (int(&x[0]), int(&x[1]))).collect()
    };
    let func_row = |x: u32| {
        let mut row = r.row(x);
        row.push(r.text(fname[&x]));
        row
    };
    match stage {
        "ownership" | "ownership_named" => {
            let v = t(if stage == "ownership" { "py_owner" } else { "py_owner_named" });
            for x in &v.rows {
                let id = int(&x[0]);
                out.push((vec![r.rank(id)], vec![V::Text(r.path.clone()), r.id(id), r.id(int(&x[1])), r.id(int(&x[2])), r.id(int(&x[3]))]));
            }
        }
        "functions" | "classes" => {
            let v = t(if stage == "functions" { "py_function" } else { "py_class" });
            for x in &v.rows {
                let id = int(&x[0]);
                let mut row = r.row(id);
                row.push(r.text(int(&x[1])));
                out.push((vec![r.rank(id)], row));
            }
        }
        "methods" => {
            let v = t("py_method");
            let cname: BTreeMap<u32, u32> = t("py_class").rows.iter().map(|x| (int(&x[0]), int(&x[1]))).collect();
            for x in &v.rows {
                let (fid, cid) = (int(&x[0]), int(&x[1]));
                let mut row = func_row(fid);
                row.push(r.id(cid));
                row.push(r.text(cname[&cid]));
                out.push((vec![r.rank(fid), r.rank(cid)], row));
            }
        }
        "attributes" => {
            for x in &t("py_attribute").rows {
                let (id, attr, obj) = (int(&x[0]), int(&x[1]), int(&x[2]));
                let mut row = r.row(id);
                row.extend([r.text(attr), r.id(obj), r.text(obj)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "calls" => {
            for x in &t("py_call").rows {
                let (id, func, callee, recv) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let mut row = r.row(id);
                row.extend([r.id(func), r.kind(func), r.text(func), r.text(callee), r.id(recv)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "parameters" => {
            for x in &t("py_parameter").rows {
                let (fid, p, name, ann) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let row = vec![V::Text(r.path.clone()), r.id(fid), r.id(p), r.kind(p), r.text(p), r.text(name), r.id(ann), x[4].clone()];
                out.push((vec![r.rank(fid), r.rank(p)], row));
            }
        }
        "assignments" => {
            for x in &t("py_assignment").rows {
                let (id, tg, val, fid) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                let mut row = r.row(id);
                row.extend([r.id(tg), r.kind(tg), r.text(tg), r.id(val), r.kind(val), r.text(val), r.id(fid)]);
                out.push((vec![r.rank(id)], row));
            }
        }
        "decorators" => {
            for x in &t("py_decorator").rows {
                let (fid, d, child, name) = (int(&x[0]), int(&x[1]), int(&x[2]), int(&x[3]));
                out.push((vec![r.rank(fid), r.rank(d), r.rank(child)], vec![V::Text(r.path.clone()), r.id(fid), r.id(d), r.text(name)]));
            }
        }
        "import_identifiers" => {
            for x in &t("py_import_identifier").rows {
                let (id, imp) = (int(&x[0]), int(&x[1]));
                out.push((vec![r.rank(imp), r.rank(id)], vec![V::Text(r.path.clone()), r.id(id)]));
            }
        }
        "contexts" => {
            let v = t("py_context");
            let c = |n: &str| col(&v, n);
            let (cn, pl, pp, ch, ph, ma) = (c("class_name"), c("planning"), c("preprocess"), c("ctx_helper"), c("provider_helper_name"), c("materializer"));
            for x in &v.rows {
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

fn check_file(rules: &[Rule], targets: &[Target], views_c: &Views, f: &Facts, path: &str, source: &str, out: &mut Out, check: bool) -> u64 {
    let mut vns = Vec::new();
    let t0 = Instant::now();
    let vdb = match views::evaluate(views_c, f, &mut vns) {
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
        out.view_rows[i] += vdb.tables[name].rows.len() as u64;
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
        for x in &py_node.rows {
            let id = int(&x[0]) as usize;
            if !seen.insert((f.kind[id], f.start[id], f.end[id])) {
                out.key_collisions += 1;
            }
        }
    }
    let pathrc: Rc<str> = Rc::from(path);
    for tg in targets {
        let rule = &rules[tg.rule];
        let Kind::Relational(plan) = &rule.clauses[tg.clause].kind else { unreachable!() };
        let mut db = Db { tables: Default::default(), source, path, regexes: Default::default() };
        let dump = plan.dump.is_some();
        if let Some(g) = &plan.dump {
            let ctx = eval::FileCtx { f, path, source };
            let mut ns = eval::DumpNs::default();
            let Some((tables, _)) = eval::dump_rows(rule, g, plan, &ctx, &mut ns) else {
                out.failures.push(format!("{path}: {}: dump refused", tg.clause_id));
                continue;
            };
            for (m, t) in g.members.iter().zip(tables) {
                db.tables.insert(plan.inputs[m.plan_input].0.to_ascii_lowercase(), Rc::new(t));
            }
        } else if let Some(spec) = &plan.syntax {
            for (n, t) in spec.tables(f, path) {
                db.tables.insert(n.to_ascii_lowercase(), Rc::new(t));
            }
        }
        for &si in &tg.run {
            let StagePlan::Query { name, stmt, .. } = &plan.stages[si] else { unreachable!() };
            match engine::execute(&db, stmt) {
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
        for (i, row) in nodes.rows.iter().enumerate() {
            match &row[idc] {
                V::Int(x) => rank[*x as usize] = i as u32,
                V::Text(s) => {
                    by_key.insert(s.to_string(), i as u32);
                }
                o => panic!("nodes id {o:?}"),
            }
        }
        let r0 = Render { f, path: pathrc.clone(), dump, nodes: nodes.clone(), rank: Vec::new() };
        // The clause's gate-true node set is N (`py_node`) or all named nodes (`py_named`).
        let py_named = vdb.tables["py_named"].clone();
        let all_named = nodes.rows.len() != py_node.rows.len();
        let set = if all_named { &py_named } else { &py_node };
        if dump {
            for x in &set.rows {
                let id = int(&x[0]);
                if let V::Text(k) = r0.id(id) {
                    if let Some(&i) = by_key.get(&*k) {
                        rank[id as usize] = i;
                    }
                }
            }
        }
        let missing = set.rows.iter().filter(|x| rank[int(&x[0]) as usize] == NONE).count();
        if missing > 0 || nodes.rows.len() != set.rows.len() {
            out.nodes_mismatch.push(format!("{path}: {}: nodes {} vs N {} / named {} ({missing} missing)", tg.clause_id, nodes.rows.len(), py_node.rows.len(), py_named.rows.len()));
            continue;
        }
        if all_named {
            *out.extra_nodes.entry(tg.clause_id.clone()).or_default() += (nodes.rows.len() - py_node.rows.len()) as u64;
        }
        let r = Render { rank, ..r0 };
        for (si, sname) in &tg.stages {
            let StagePlan::Query { stmt, .. } = &plan.stages[*si] else { unreachable!() };
            let Some(reference) = db.tables.get(&sname.to_ascii_lowercase()).cloned() else { continue };
            let view_stage = if all_named && sname == "ownership" { "ownership_named" } else { sname.as_str() };
            let mut rows = render(view_stage, &vdb, &r);
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            let expected: Vec<Box<[V]>> = rows.into_iter().map(|(_, row)| row.into_boxed_slice()).collect();
            let expected = views::store(Table { schema: stmt.schema().clone(), rows: expected }, stmt.schema()).rows;
            let st = out.stats.entry((sname.clone(), tg.clause_id.clone())).or_default();
            st.files += 1;
            st.rows += reference.rows.len() as u64;
            let mut a: Vec<String> = expected.iter().map(|x| key(x)).collect();
            let mut b: Vec<String> = reference.rows.iter().map(|x| key(x)).collect();
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
    }
    views_ns
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
    let t_all = Instant::now();
    let next = AtomicUsize::new(0);
    let outs: Vec<Out> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                let (paths, next, policies, shared) = (&paths, &next, &policies, &shared);
                s.spawn(move || {
                    let (rules, targets) = if check { compile(policies, shared) } else { (Vec::new(), Vec::new()) };
                    let vc = Views::compile().unwrap();
                    let mut out = Out::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(i) else { break };
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let source = Arc::new(src);
                        let mut ns = facts::BuildNs::default();
                        let Ok(facts::Built::Facts(f)) = facts::build_timed(path, source.clone(), &mut ns) else { continue };
                        let vns = check_file(&rules, &targets, &vc, &f, path, &source, &mut out, check);
                        let mut tns = 0;
                        if typedefs {
                            let t = Instant::now();
                            match f.typedefs() {
                                Ok(d) => out.annotations += d.len() as u64,
                                Err(e) => out.failures.push(format!("{path}: typedefs: {e}")),
                            }
                            tns = t.elapsed().as_nanos() as u64;
                        }
                        out.per_file.push((i as u32, source.len() as u32, f.len() as u32, ns.parse, ns.extract, vns, tns));
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
    }
    per_file.sort();
    let mut tsv = String::from("path\tbytes\tnodes\tparse_ns\textract_ns\tviews_ns\ttypedefs_ns\n");
    for p in &per_file {
        tsv.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", paths[p.0 as usize], p.1, p.2, p.3, p.4, p.5, p.6));
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
        "annotations": annotations,
    });
    std::fs::write(Path::new(&out_dir).join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&json!({"files": res["files"], "wall_s": wall, "bag_mismatch_total": total_bag, "order_mismatch_total": total_order, "failures": res["failures"].as_array().map(|a| a.len()), "nodes_mismatch": res["nodes_mismatch"].as_array().map(|a| a.len()), "collisions": collisions, "skipped": skipped, "views_ns": res["views_ns"], "parse_extract_ns": res["parse_extract_ns"]})).unwrap());
}
