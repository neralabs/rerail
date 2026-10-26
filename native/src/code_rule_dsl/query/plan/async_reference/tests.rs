use super::native_oracle::Native;
use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    tape::{Grammar, TapeDoc, TapeImage},
    test_support::CAPABILITIES,
};
use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
use serde_json::{Value, json};
use std::collections::HashMap;

fn parsed(sources: &[String]) -> Vec<AstGrep<TapeDoc>> {
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    sources
        .iter()
        .map(|s| {
            let doc = StrDoc::new(s, GrammarDialect::Python);
            let image = TapeImage::encode(doc.tree.root_node(), &grammar, s, &|| false).unwrap();
            AstGrep::doc(
                TapeDoc::restore(
                    std::sync::Arc::new(s.clone()),
                    std::sync::Arc::clone(&grammar),
                    &image,
                    &|| false,
                )
                .unwrap(),
            )
        })
        .collect()
}
fn reference_run(sources: &[String], copies: usize) -> Run {
    let parsed = parsed(sources);
    let roots: Vec<_> = parsed
        .iter()
        .enumerate()
        .map(|(doc, root)| (doc as u32, root.root()))
        .collect();
    execute(&roots, &BTreeMap::new(), copies)
}
fn native_policy() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    let mut combined: Value = serde_json::from_str(POLICIES[0]).unwrap();
    combined["id"] = json!("complete-async-family-reference-oracle");
    combined["clauses"] = Value::Array(
        POLICIES
            .iter()
            .map(|text| {
                let policy: Value = serde_json::from_str(text).unwrap();
                let mut clause = policy["clauses"][0].clone();
                // This gate compares raw matching. Suppression and payload integration
                // are a later gate and cannot hide a matching disagreement.
                clause["violation"]
                    .as_object_mut()
                    .unwrap()
                    .remove("suppress");
                clause
            })
            .collect(),
    );
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    capabilities["limits"]["maxFiles"] = json!(10000);
    capabilities["limits"]["maxSourceBytes"] = json!(200_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(100_000_000);
    capabilities["limits"]["maxWitnesses"] = json!(1_000_000);
    CodeRuleDslRuntime::compile(
        combined.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap()
}
fn native_run(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    sources: &[String],
) -> BTreeSet<(String, u32, usize)> {
    let paths: Vec<_> = (0..sources.len())
        .map(|doc| format!("doc{doc}.py"))
        .collect();
    native_run_at(compiled, sources, &paths, &vec![None; sources.len()])
}
fn native_run_at(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    sources: &[String],
    paths: &[String],
    modules: &[Option<Value>],
) -> BTreeSet<(String, u32, usize)> {
    let files: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(doc, source)| {
            let mut file = json!({"path":paths[doc],"language":"python","source":source});
            if let Some(module) = &modules[doc] {
                file["module"] = module.clone();
            }
            file
        })
        .collect();
    let ids: HashMap<_, _> = paths
        .iter()
        .enumerate()
        .map(|(id, path)| (path.as_str(), id as u32))
        .collect();
    let snapshot =
        json!({"format":"rerail.repository-snapshot/v1","revision":0,"facts":[],"files":files});
    let output: Value = serde_json::from_str(
        &compiled
            .evaluate(snapshot.to_string(), "report".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(output["failures"], json!([]), "{output}");
    output["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let path = v["path"].as_str().unwrap();
            let doc = ids[path];
            let start = v["span"]["startByte"].as_u64().unwrap() as usize;
            let line = sources[doc as usize].as_bytes()[..start]
                .iter()
                .filter(|&&c| c == b'\n')
                .count()
                + 1;
            (v["code"].as_str().unwrap().to_owned(), doc, line)
        })
        .collect()
}
fn keys(run: &Run) -> BTreeSet<(String, u32, usize)> {
    run.findings
        .keys
        .iter()
        .map(|k| (k.code.clone(), k.doc, k.line))
        .collect()
}
fn check(sources: Vec<String>) -> Run {
    let run = reference_run(&sources, 1);
    assert!(
        run.findings
            .coverage
            .values()
            .all(|c| *c == Coverage::Complete),
        "{:?}",
        run.findings.coverage
    );
    let compiled = native_policy();
    let native = native_run(&compiled, &sources);
    let tapes = parsed(&sources);
    let paths: Vec<_> = (0..sources.len())
        .map(|doc| format!("doc{doc}.py"))
        .collect();
    let modules = BTreeMap::new();
    let indexed = Native::new(&tapes, &compiled, &paths, &modules);
    let generated = generated_shape::queries();
    let generated_roots: Vec<_> = tapes
        .iter()
        .enumerate()
        .map(|(i, tape)| (i as u32, tape.root()))
        .collect();
    let generated_run = execute_sql(
        &generated_roots,
        &modules,
        1,
        &[&generated[0].sql, &generated[1].sql],
    );
    assert_eq!(
        keys(&generated_run),
        native,
        "generated shape must match both oracles"
    );
    assert_eq!(generated_run.findings.coverage, run.findings.coverage);
    assert_eq!(
        generated_run.metrics.semantic_requests,
        run.metrics.semantic_requests
    );
    assert!(
        generated_run
            .metrics
            .filters
            .iter()
            .all(|(name, &count)| count <= if name == "receiver" { 2 } else { 1 }),
        "generated shape must retain batch boundaries"
    );
    assert_eq!(
        native,
        indexed.run().keys,
        "indexed native must match stateless native"
    );
    assert_eq!(
        keys(&run),
        native,
        "reference-only {:?}; native-only {:?}",
        keys(&run).difference(&native).collect::<Vec<_>>(),
        native.difference(&keys(&run)).collect::<Vec<_>>()
    );
    assert_eq!(run.metrics.sql_executions, 2);
    assert!(
        run.metrics
            .filters
            .iter()
            .all(|(name, &count)| count <= if name == "receiver" { 2 } else { 1 }),
        "{:?}",
        run.metrics.filters
    );
    run
}
#[test]
fn complete_async_family_matches_native_on_binding_and_context_edges() {
    check(vec![
        r#"
import requests as rq
import httpx
import os
import subprocess
from pathlib import Path as P
from os import P_WAIT as WAIT
from io import open as io_open
async def outer():
    rq.get('url')
    (rq.get # comment between callee and arguments
        ('url'))
    (\
      httpx.post\
    )('url')
    subprocess.run(['cmd'])
    os.waitpid(1, 0)
    os.spawnl()
    os.spawnl(os.P_WAIT, 'x')
    os.spawnl(mode=WAIT)
    os.spawnl(os.P_NOWAIT, 'x')
    os.spawnl(*args)
    os.spawnl(*args, mode=os.P_WAIT)
    os.spawnl(**kwargs)
    m = os.P_WAIT
    os.spawnl(m, 'x')
    m, other = (os.P_WAIT, None)
    os.spawnl(m, 'x')
    a, a = (None, os.P_WAIT)
    os.spawnl(a, 'x')
    m, other = (os.P_WAIT)
    os.spawnl(m, 'x')
    a, a = (os.P_WAIT, None)
    os.spawnl(a, 'x')
    open('x')
    io_open('x')
    P('x').open()
    (P # comment between constructor and arguments
        ('x')).open()
    P('x').ｏｐｅｎ()
    (P('x')).open()
    p = P('x')
    p.open()
    (p).open()
    p: object = P('x')
    p.open()
    p, q = (P('x'), None)
    p.open()
    q.open()
    p, q = {P('x'), None}
    p.open()
    p, q = (P('x'))
    p.open()
    with P('x') as w:
        w.open()
    with P('x') as (w):
        w.open()
    with P('x') as (w, other):
        w.open()
    def sync(default=rq.get('default')):
        rq.get('sync')
    class C(base(rq.get('base'))):
        rq.get('class')
        async def inside(self):
            rq.get('inside')
    f = lambda x=rq.get('default'): rq.get('lambda')
    values = (rq.get(x) for x in seq)
async def shadow(open, rq, P):
    open('x')
    rq.get('x')
    P('x').open()
"#
        .to_owned(),
    ]);
}
#[test]
fn complete_async_family_matches_existing_path_fixtures() {
    check(vec![
        include_str!("../../../../../../fixtures/python/value/ruff/ASYNC230.py").into(),
        include_str!("../../../../../../fixtures/python/value/ruff/ASYNC230-unpacking.py").into(),
    ]);
}
#[test]
fn primary_screen_preserves_receiver_demands_and_skips_irrelevant_scopes() {
    let irrelevant = reference_run(&["import unrelated\nasync def f():\n unrelated.run()\n factory().get()\n items[0].post()\n".into()], 1);
    assert_eq!(irrelevant.metrics.admitted_primary_refs, 0);
    assert_eq!(irrelevant.metrics.scope_builds, 0);
    assert_eq!(irrelevant.metrics.semantic_requests, 0);
    assert!(irrelevant.findings.keys.is_empty());
    let receiver = check(vec![
        "from pathlib import Path as P\nasync def f():\n P('x').open()\n p = P('x')\n p.open()\n"
            .into(),
    ]);
    assert_eq!(receiver.metrics.admitted_primary_refs, 0);
    assert!(receiver.metrics.semantic_requests > 0);
    assert_eq!(receiver.findings.keys.len(), 2);
}
#[test]
fn batched_positive_documents_preserve_all_four_labels() {
    let sources = (0..24).map(|i| format!(
        "import requests as r\nimport os as o\nfrom pathlib import Path as P\nasync def f{i}():\n r.get('url')\n o.system('cmd')\n o.spawnl(o.P_WAIT, 'x')\n o.waitpid(1, 0)\n P('x').open()\n"
    )).collect();
    let run = check(sources);
    assert_eq!(run.findings.keys.len(), 120);
    assert_eq!(run.metrics.scope_builds, 24);
}
#[test]
fn duplicate_labels_do_not_repeat_producer_work() {
    let sources=vec!["import os\nfrom pathlib import Path\nasync def f():\n os.spawnl(os.P_WAIT, 'x')\n Path('x').open()\n".into()];
    let one = reference_run(&sources, 1);
    let many = reference_run(&sources, 16);
    assert_eq!(
        one.findings.keys.len(),
        2,
        "sharing gate must exercise positive output"
    );
    assert!(one.metrics.semantic_requests > 0);
    assert_eq!(one.metrics.syntax_nodes, many.metrics.syntax_nodes);
    assert_eq!(
        one.metrics.semantic_requests,
        many.metrics.semantic_requests
    );
    assert_eq!(
        one.metrics.qualified_lookups,
        many.metrics.qualified_lookups
    );
    assert_eq!(one.metrics.binding_lookups, many.metrics.binding_lookups);
    assert_eq!(one.metrics.scope_builds, many.metrics.scope_builds);
    assert_eq!(one.metrics.origin_rows, many.metrics.origin_rows);
    assert_eq!(one.findings.keys.len() * 16, many.findings.keys.len());
}
#[test]
fn failed_document_does_not_publish_provisional_findings() {
    let run = reference_run(
        &[
            "import os\nasync def f():\n os.wait()\n !!! broken\n".into(),
            "import os\nasync def f():\n os.wait()\n".into(),
        ],
        1,
    );
    assert!(run.findings.keys.iter().all(|k| k.doc == 1));
    assert_eq!(run.findings.keys.len(), 1);
    assert!(
        run.findings
            .coverage
            .iter()
            .filter(|((_, doc), _)| *doc == 0)
            .all(|(_, c)| matches!(c, Coverage::Unknown(_)))
    );
}

fn collect_paths(root: &std::path::Path, paths: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().unwrap() != ".git" {
                collect_paths(&path, paths);
            }
        } else if path.extension().is_some_and(|ext| ext == "py") {
            paths.push(path);
        }
    }
}
#[test]
#[ignore = "requires RERAIL_SHARED_CORPUS; complete family, not whole catalog"]
fn complete_async_family_corpus_profile() {
    let root = std::env::var("RERAIL_SHARED_CORPUS").expect("RERAIL_SHARED_CORPUS");
    let mut paths = Vec::new();
    collect_paths(std::path::Path::new(&root), &mut paths);
    paths.sort();
    let limit = std::env::var("RERAIL_SHARED_LIMIT")
        .ok()
        .map(|x| x.parse().unwrap())
        .unwrap_or(usize::MAX);
    paths.truncate(limit);
    assert!(!paths.is_empty());
    let sources: Vec<_> = paths
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    let module_wires: Vec<_> = paths
        .iter()
        .map(|path| crate::code_rule_dsl::model::python_module_path_on_disk(path))
        .collect();
    let paths: Vec<_> = paths
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let modules: BTreeMap<_, _> = module_wires
        .iter()
        .enumerate()
        .filter_map(|(doc, value)| {
            let module = match value {
                Some(Value::String(s)) => ModulePath::dotted(s),
                Some(Value::Array(names)) => ModulePath::names(
                    &names
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>(),
                ),
                None => ModulePath::stem(&paths[doc]),
                _ => unreachable!(),
            };
            module.map(|module| (doc as u32, module))
        })
        .collect();
    let start = cpu_ns();
    let parsed = parsed(&sources);
    let parse_cpu = cpu_ns() - start;
    let roots: Vec<_> = parsed
        .iter()
        .enumerate()
        .map(|(doc, tree)| (doc as u32, tree.root()))
        .collect();
    let compile = cpu_ns();
    let native = native_policy();
    let compile_cpu = cpu_ns() - compile;
    let indexed = Native::new(&parsed, &native, &paths, &modules);
    let compile = cpu_ns();
    let generated = generated_shape::queries();
    let generated_compile_cpu = cpu_ns() - compile;
    let generated_queries = [&*generated[0].sql, &*generated[1].sql];
    let reference_queries = [SELECT_TARGETS, COMPOSE];
    let mut runs = Vec::new();
    for iteration in 0..3 {
        let candidate = |queries: &[&str; 2]| {
            let cpu = cpu_ns();
            let wall = Instant::now();
            let mut run = execute_sql(&roots, &modules, 1, queries);
            // Include producer/table destruction in job CPU. EXPLAIN remains
            // diagnostic work, while preparation is reported separately.
            run.metrics.execution_ns =
                cpu_ns() - cpu - run.metrics.preparation_ns - run.metrics.diagnostics_ns;
            run.metrics.wall_ns =
                wall.elapsed().as_nanos() as u64 - run.metrics.diagnostics_wall_ns;
            run
        };
        let baseline = || indexed.run();
        let (run, oracle, generated_run) = match iteration % 3 {
            0 => (
                candidate(&reference_queries),
                baseline(),
                candidate(&generated_queries),
            ),
            1 => {
                let native = baseline();
                let generated = candidate(&generated_queries);
                (candidate(&reference_queries), native, generated)
            }
            _ => {
                let generated = candidate(&generated_queries);
                (candidate(&reference_queries), baseline(), generated)
            }
        };
        assert_eq!(
            keys(&generated_run),
            keys(&run),
            "generated/reference parity"
        );
        assert_eq!(generated_run.findings.coverage, run.findings.coverage);
        assert_eq!(
            generated_run.metrics.semantic_requests,
            run.metrics.semantic_requests
        );
        assert_eq!(generated_run.metrics.sql_executions, 2);
        assert!(
            generated_run
                .metrics
                .filters
                .iter()
                .all(|(name, &count)| count <= if name == "receiver" { 2 } else { 1 }),
            "generated per-row scan: {:?}",
            generated_run.metrics.filters
        );
        let mismatches = keys(&run)
            .symmetric_difference(&oracle.keys)
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            mismatches.is_empty(),
            "first discrepancies: {:?}",
            mismatches.iter().take(30).collect::<Vec<_>>()
        );
        assert!(
            run.findings
                .coverage
                .values()
                .all(|c| *c == Coverage::Complete),
            "incomplete reference domains"
        );
        assert!(run.metrics.sql_executions <= 100, "hard execution budget");
        runs.push(json!({"iteration":iteration,
            "reference_job_cpu_ns":run.metrics.execution_ns+run.metrics.preparation_ns+parse_cpu+indexed.index_ns,
            "native_job_cpu_ns":oracle.cpu_ns+oracle.compile_ns+parse_cpu+indexed.index_ns,
            "execution_cpu_ratio":run.metrics.execution_ns as f64/oracle.cpu_ns as f64,
            "generated_to_reference_cpu_ratio":generated_run.metrics.execution_ns as f64/run.metrics.execution_ns as f64,
            "generated_job_cpu_ns":generated_run.metrics.execution_ns+generated_run.metrics.preparation_ns+parse_cpu+indexed.index_ns,
            "generated":generated_run.metrics,
            "reference":run.metrics,"findings":run.findings.keys.len(),
            "native_execution_cpu_ns":oracle.cpu_ns,"native_execution_wall_ns":oracle.wall_ns,
            "native_selection_cpu_ns":oracle.prepare_ns,"native_sql_compile_ns":oracle.compile_ns,
            "native_roots_matched":oracle.roots_matched,
            "native_frontier_cached":iteration>0,
            "native_match_cpu_ns":oracle.cpu_ns.saturating_sub(oracle.prepare_ns),
            "reference_to_native_match_ratio":run.metrics.execution_ns as f64/oracle.cpu_ns.saturating_sub(oracle.prepare_ns) as f64}));
    }
    let mut ratios: Vec<_> = runs
        .iter()
        .map(|run| run["reference_to_native_match_ratio"].as_f64().unwrap())
        .collect();
    ratios.sort_by(f64::total_cmp);
    let mut generated_ratios: Vec<_> = runs
        .iter()
        .map(|run| run["generated_to_reference_cpu_ratio"].as_f64().unwrap())
        .collect();
    generated_ratios.sort_by(f64::total_cmp);
    let gates = json!({
        "native_finding_parity": true,
        "complete_domains": true,
        "execution_sql_budget": 100,
        "execution_sql_budget_pass": runs.iter().all(|run| run["reference"]["sql_executions"].as_u64().unwrap() <= 100),
        "median_reference_to_native_match_cpu_ratio": ratios[ratios.len()/2],
        "reference_2x_cpu_gate_pass": ratios[ratios.len()/2] <= 0.5,
        "generated_shape_to_reference_cpu_ratio": generated_ratios[generated_ratios.len()/2],
        "generated_shape_1_25x_cpu_gate_pass": generated_ratios[generated_ratios.len()/2] <= 1.25,
        "generated_compiler_qualified": false,
        "production_cutover": false
    });
    let report = json!({"gates":gates,"scope":"complete four-rule async family; raw finding keys, no suppressions/payloads; indexed native shares scope/memo across clauses; job totals add common parse+tape and mandatory index construction; native candidate frontier is fresh only in iteration0, cached thereafter; compare match CPU separately",
        "corpus":root,"files":sources.len(),"source_bytes":sources.iter().map(String::len).sum::<usize>(),
        "parse_tape_cpu_ns":parse_cpu,"index_construction_cpu_ns":indexed.index_ns,
        "native_compile_cpu_ns":compile_cpu,"generated_shape_compile_cpu_ns":generated_compile_cpu,
        "sql":[SELECT_TARGETS,COMPOSE],"generated_sql":generated_queries,
        "sql_bytes":[SELECT_TARGETS.len(),COMPOSE.len()],"generated_sql_bytes":generated_queries.map(str::len),"runs":runs});
    eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("RERAIL_SHARED_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
