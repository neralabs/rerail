//! Ignored real-corpus experiment. It reports every refusal; unsupported clauses never silently
//! use the old scheduler. This measures the matching kernel, not the workspace/render pipeline.
use super::{
    adapter::Compiler,
    graph::Policy,
    run,
    tests::{columns, decode},
};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    language::GrammarDialect,
    model::{ClauseKind, Query},
    module_path::ModulePath,
    sql::tests::differential::tape,
    sql::{self, emit, exec::FileContext, lower::ProgramBuilder},
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Instant};

#[test]
#[ignore = "whole-call matching-kernel prototype; set RERAIL_WHOLE_CALL_CONFIG and RERAIL_WHOLE_CALL_OUTPUT"]
fn whole_call_corpus() {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(workload)
        .unwrap()
        .join()
        .unwrap();
}
fn workload() {
    let config_path = std::env::var("RERAIL_WHOLE_CALL_CONFIG").expect("workload config");
    let output = std::env::var("RERAIL_WHOLE_CALL_OUTPUT").expect("artifact directory");
    let out = Path::new(&output);
    std::fs::create_dir_all(out).unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    let capabilities = std::fs::read_to_string(config["capabilities"].as_str().unwrap()).unwrap();
    let policies: Vec<_> = config["policies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            CodeRuleDslRuntime::compile(
                std::fs::read_to_string(p.as_str().unwrap()).unwrap(),
                "json".into(),
                capabilities.clone(),
            )
            .unwrap()
        })
        .collect();
    let mut grammars = Vec::new();
    let mut labels = Vec::new();
    let mut each = Vec::new();
    for policy in &policies {
        for clause in &policy.inner.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for g in matchers
                    .iter()
                    .filter(|g| g.language == GrammarDialect::Python && g.applies)
                {
                    grammars.push(g);
                    labels.push(format!("{}/{}", policy.inner.id, clause.id));
                    each.push(clause.violation.each.is_some());
                }
            }
        }
    }
    let corpus = Path::new(config["corpus"].as_str().unwrap());
    let docs: Vec<_> = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let relative = p.as_str().unwrap();
            let absolute = corpus.join(relative);
            let module = crate::code_rule_dsl::language::adapter::module_path("python", &absolute)
                .and_then(|value| match value {
                    Value::String(s) => ModulePath::dotted(&s),
                    Value::Array(a) => {
                        ModulePath::names(&a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .map(Arc::new);
            (
                tape(
                    GrammarDialect::Python,
                    &std::fs::read_to_string(absolute).unwrap(),
                ),
                FileContext {
                    path: Some(relative.into()),
                    module,
                    ..FileContext::default()
                },
            )
        })
        .collect();
    let files = format!(
        "SELECT CAST(key AS INTEGER) FROM json_each('{}')",
        json!(vec![0; docs.len()])
    );
    let started = Instant::now();
    let mut builder = ProgramBuilder::default();
    let mut compiler = Compiler::default();
    let mut admitted = Vec::new();
    let mut statements = Vec::new();
    let mut outputs = Vec::new();
    let mut coverage = Vec::new();
    for (index, g) in grammars.iter().enumerate() {
        if each[index] {
            coverage.push(json!({"clause":labels[index],"supported":false,"reason":"violation.each enumeration"}));
            continue;
        }
        let statement = match builder.clause_each(g, None) {
            Ok(s) => s,
            Err(e) => {
                coverage
                    .push(json!({"clause":labels[index],"supported":false,"reason":e.to_string()}));
                continue;
            }
        };
        let features = super::recursive::features(&statement);
        let before = compiler.clone();
        match compiler.emitted(
            &emit::clause(&statement),
            &columns(&statement),
            &builder.stages,
            &files,
        ) {
            Ok(id) => {
                outputs.push((admitted.len(), id));
                admitted.push(*g);
                statements.push(statement);
                coverage.push(json!({"clause":labels[index],"supported":true,"relation":id,"features":features}));
            }
            Err(error) => {
                compiler = before;
                coverage.push(json!({"clause":labels[index],"supported":false,"reason":error,"features":features}));
            }
        }
    }
    let compile_ms = started.elapsed().as_secs_f64() * 1000.;
    std::fs::write(
        out.join("coverage.json"),
        serde_json::to_vec_pretty(&coverage).unwrap(),
    )
    .unwrap();
    eprintln!(
        "whole-call census: {} / {} clauses admitted, {} relations, {:.1} ms compile",
        admitted.len(),
        grammars.len(),
        compiler.graph.relations.len(),
        compile_ms
    );
    assert!(!admitted.is_empty());
    let legacy = sql::exec::compile(&admitted).unwrap();
    let mut session = legacy.session().unwrap();
    let runs: Vec<_> = (0..admitted.len())
        .map(|statement| sql::ClauseRun {
            statement,
            files: (0..docs.len()).map(|file| (file, None)).collect(),
        })
        .collect();
    let started = crate::code_rule_dsl::sql::timing::Started::now();
    let expected = session.run(docs.clone(), &runs).unwrap();
    let legacy_time = started.elapsed();
    let graph = compiler.graph;
    let mut alternatives = Vec::new();
    for (mode, policy) in [
        ("cte", Policy::All),
        ("cte", Policy::Reuse),
        ("cte", Policy::Boundaries),
        ("buffers", Policy::All),
        ("buffers", Policy::Reuse),
        ("buffers", Policy::Boundaries),
    ] {
        let mut fresh = ProgramBuilder::default();
        for (index, g) in grammars.iter().enumerate() {
            if !each[index] {
                let _ = fresh.clause_each(g, None);
            }
        }
        let (physical, sql) = if mode == "cte" {
            (graph.clone(), graph.render(&outputs, policy))
        } else {
            super::buffers::render(&graph, &outputs, policy)
        };
        std::fs::write(out.join(format!("{mode}-{policy:?}.sql")), &sql).unwrap();
        let started = crate::code_rule_dsl::sql::timing::Started::now();
        let mut record = sql::exec::with_terminals(
            docs.clone(),
            fresh.terminals,
            |connection, _, frames| match run::execute(connection, frames, &physical, &sql) {
                Ok(got) => {
                    let mut actual =
                        vec![
                            vec![Vec::new(); config["files"].as_array().unwrap().len()];
                            admitted.len()
                        ];
                    for (clause, file, root, values) in &got.hits {
                        actual[*clause][*file].push(decode(&statements[*clause], *root, values));
                    }
                    let parity = actual == expected;
                    if !parity {
                        std::fs::write(
                            out.join(format!("{policy:?}-mismatch.txt")),
                            format!("expected {expected:#?}\nactual {actual:#?}"),
                        )
                        .unwrap();
                    }
                    json!({"mode":mode,"policy":format!("{policy:?}"),"parity":parity,"matches":got.hits.len(),"metrics":got.metrics})
                }
                Err(e) => {
                    json!({"mode":mode,"policy":format!("{policy:?}"),"error":e.to_string(),"parity":null,"status":"rejected"})
                }
            },
        );
        let total = started.elapsed();
        record["wholeKernelCpuNs"] = json!(total.cpu);
        record["wholeKernelWallNs"] = json!(total.wall);
        eprintln!("whole-call corpus {record}");
        alternatives.push(record);
    }
    let report = json!({"scope":"matching kernel; all roots of each admitted grammar in all configured files; no policy selectors, rendering, or cross-file policy composition", "files":config["files"],"policies":policies.len(),"clauses":grammars.len(),"admitted":admitted.len(),"compileWallMs":compile_ms,"estimatedMaxExpandedReferences":graph.expanded_references(),"legacySingleBatch":{"cpuNs":legacy_time.cpu,"wallNs":legacy_time.wall,"statements":session.stats().statements},"alternatives":alternatives});
    let mismatches = report["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["parity"] == false);
    std::fs::write(
        out.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        !mismatches,
        "a completed whole-call plan changed roots or captures; inspect results.json"
    );
}
