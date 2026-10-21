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
    let files = "SELECT file FROM node WHERE id=0".to_owned();
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
    if std::env::var("RERAIL_WHOLE_CALL_CASES")
        .is_ok_and(|s| s.split(',').any(|s| s == "recursive-machine"))
    {
        recursive_corpus(out, &grammars, &labels, &each, &docs);
    }
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
    for (mode, policy, budget, phases) in [
        ("cte", Policy::All, 0, 1),
        ("cte", Policy::Reuse, 0, 1),
        ("cte", Policy::Boundaries, 0, 1),
        ("buffers", Policy::All, 0, 1),
        ("buffers", Policy::Reuse, 0, 1),
        ("buffers", Policy::Boundaries, 0, 1),
        ("shaped-64-1", Policy::All, 64, 1),
        ("shaped-8-1", Policy::All, 8, 1),
        ("shaped-16-1", Policy::All, 16, 1),
        ("shaped-exists-64-1", Policy::All, 64, 1),
        ("shaped-256-1", Policy::All, 256, 1),
        ("shaped-64-4", Policy::All, 64, 4),
        ("shaped-256-4", Policy::All, 256, 4),
    ] {
        let case = format!("{mode}-{policy:?}");
        if let Ok(selected) = std::env::var("RERAIL_WHOLE_CALL_CASES")
            && !selected.split(',').any(|s| s == case)
        {
            continue;
        }
        let mut fresh = ProgramBuilder::default();
        for (index, g) in grammars.iter().enumerate() {
            if !each[index] {
                let _ = fresh.clause_each(g, None);
            }
        }
        let generated = crate::code_rule_dsl::sql::timing::Started::now();
        let (physical, queries, planning) = if budget > 0 {
            let shape = if mode.contains("exists") {
                super::shapes::StageProbe::Exists
            } else {
                super::shapes::StageProbe::RowIn
            };
            let plan = super::physical::plan_with_shape(&graph, &outputs, budget, phases, shape);
            (plan.graph, plan.queries, plan.metadata)
        } else if mode == "cte" {
            (
                graph.clone(),
                vec![graph.render(&outputs, policy)],
                Value::Null,
            )
        } else {
            let (physical, sql) = super::buffers::render(&graph, &outputs, policy);
            (physical, vec![sql], Value::Null)
        };
        let generation = generated.elapsed();
        for (part, sql) in queries.iter().enumerate() {
            std::fs::write(out.join(format!("{case}-{part}.sql")), sql).unwrap();
        }
        let profiling = std::env::var_os("RERAIL_WHOLE_CALL_PROFILE").is_some();
        let started = crate::code_rule_dsl::sql::timing::Started::now();
        let mut observation = Value::Null;
        let mut record = sql::exec::with_terminals(
            docs.clone(),
            fresh.terminals,
            |connection, _, frames| match run::execute_observed(
                connection,
                frames,
                &physical,
                &queries,
                profiling,
                Some(&mut observation),
            ) {
                Ok(mut got) => {
                    if profiling {
                        let owners: Vec<_> = coverage
                            .iter()
                            .filter(|c| c["supported"] == true)
                            .map(|c| {
                                (
                                    c["relation"].as_u64().unwrap() as usize,
                                    c["clause"].as_str().unwrap().to_owned(),
                                )
                            })
                            .collect();
                        super::profile::summarize(&mut got.metrics, &graph, &owners);
                    }
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
                            out.join(format!("{case}-mismatch.txt")),
                            format!("expected {expected:#?}\nactual {actual:#?}"),
                        )
                        .unwrap();
                    }
                    json!({"mode":mode,"policy":format!("{policy:?}"),"parity":parity,"matches":got.hits.len(),"metrics":got.metrics,"planning":planning})
                }
                Err(e) => {
                    json!({"mode":mode,"policy":format!("{policy:?}"),"error":e.to_string(),"parity":null,"status":"rejected","metrics":observation})
                }
            },
        );
        let total = started.elapsed();
        record["wholeKernelCpuNs"] = json!(
            total
                .cpu
                .saturating_sub(record["metrics"]["diagnosticCpuNs"].as_u64().unwrap_or(0))
        );
        record["wholeKernelWallNs"] = json!(total.wall);
        record["sqlGenerationCpuNs"] = json!(generation.cpu);
        record["sqlGenerationWallNs"] = json!(generation.wall);
        eprintln!(
            "whole-call corpus {case}: parity={} error={} cpu={} vm={} publications={}",
            record["parity"],
            record["error"],
            record["metrics"]["cpuNs"],
            record["metrics"]["vmSteps"],
            record["metrics"]["publishedRows"]
        );
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

fn recursive_corpus(
    out: &Path,
    grammars: &[&crate::code_rule_dsl::matcher::GrammarMatcher],
    labels: &[String],
    each: &[bool],
    docs: &[(crate::code_rule_dsl::tape::TapeDoc, FileContext)],
) {
    use super::recursive_machine;
    use crate::code_rule_dsl::sql::{
        exec::{Captured, SqlMatch},
        plan::SlotKind,
    };
    let mut builder = ProgramBuilder::default();
    let mut admitted = Vec::new();
    let mut statements = Vec::new();
    let mut coverage = Vec::new();
    for (i, grammar) in grammars.iter().enumerate() {
        if each[i] {
            coverage.push(
                json!({"clause":labels[i],"supported":false,"reason":"violation.each enumeration"}),
            );
            continue;
        }
        let result = builder
            .clause_each(grammar, None)
            .map_err(|e| e.to_string())
            .and_then(|s| {
                recursive_machine::compile_batch_with_contract(&[&s], &builder.terminals).map(|_| s)
            });
        match result {
            Ok(s) => {
                coverage.push(json!({"clause":labels[i],"supported":true,"features":super::recursive::features(&s)}));
                admitted.push(*grammar);
                statements.push(s);
            }
            Err(e) => coverage.push(json!({"clause":labels[i],"supported":false,"reason":e})),
        }
    }
    std::fs::write(
        out.join("recursive-coverage.json"),
        serde_json::to_vec_pretty(&coverage).unwrap(),
    )
    .unwrap();
    if statements.is_empty() {
        return;
    }
    let query = recursive_machine::compile_batch_with_contract(
        &statements.iter().collect::<Vec<_>>(),
        &builder.terminals,
    )
    .unwrap();
    std::fs::write(out.join("recursive.sql"), &query).unwrap();
    let mut session = sql::exec::compile(&admitted).unwrap().session().unwrap();
    let runs: Vec<_> = (0..admitted.len())
        .map(|statement| sql::ClauseRun {
            statement,
            files: (0..docs.len()).map(|file| (file, None)).collect(),
        })
        .collect();
    let expected = session.run(docs.to_vec(), &runs);
    let profiling = std::env::var_os("RERAIL_WHOLE_CALL_PROFILE").is_some();
    let report = sql::exec::with_terminals(
        docs.to_vec(),
        builder.terminals,
        |connection, batch, frames| {
            recursive_machine::register_text(connection, batch).unwrap();
            let mut observation = Value::Null;
            match run::execute_observed(
                connection,
                frames,
                &super::graph::Graph::default(),
                &[query],
                profiling,
                Some(&mut observation),
            ) {
                Err(e) => {
                    json!({"status":"rejected","error":e.to_string(),"parity":null,"clauses":statements.len(),"files":docs.len(),"metrics":observation})
                }
                Ok(got) => {
                    let faults: Vec<_> = got.hits.iter().filter(|h| h.3["fault"] != 0).collect();
                    let mut actual = vec![vec![Vec::new(); docs.len()]; statements.len()];
                    for (rule, file, root, answer) in &got.hits {
                        if answer["fault"] != 0 || answer["result"] != 1 {
                            continue;
                        }
                        let captures = statements[*rule]
                            .slots
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.scope == 0)
                            .filter_map(|(id, slot)| {
                                answer["env"][id.to_string()].as_i64().map(|v| {
                                    (
                                        slot.name.clone(),
                                        match slot.kind {
                                            SlotKind::Node => Captured::Node(v as u32),
                                            SlotKind::Count => Captured::Count(v),
                                            _ => unreachable!(),
                                        },
                                    )
                                })
                            })
                            .collect();
                        actual[*rule][*file].push(SqlMatch {
                            root: *root,
                            captures,
                        });
                    }
                    let parity = match &expected {
                        Ok(e) => faults.is_empty() && *e == actual,
                        Err(_) => faults.iter().any(|h| h.3["fault"] == 1),
                    };
                    json!({"status":"completed","parity":parity,"clauses":statements.len(),"files":docs.len(),"terminalRootVerdicts":got.hits.len(),"faults":faults,"metrics":got.metrics})
                }
            }
        },
    );
    eprintln!(
        "recursive corpus: {} clauses, parity={}, status={}, error={}",
        statements.len(),
        report["parity"],
        report["status"],
        report["error"]
    );
    std::fs::write(
        out.join("recursive-results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_ne!(
        report["parity"], false,
        "recursive corpus roots/captures differ"
    );
}
