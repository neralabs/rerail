use super::super::{failure, tests::ExecutionTrace};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    matcher::{self, Stopped, value_probe},
    sql::{exec, profiling, tests::differential::tape},
    test_support::CAPABILITIES,
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

pub(super) fn policy(rules: &[Value]) -> CompiledCodeRuleDsl {
    policy_utils(rules, json!({}))
}

pub(super) fn policy_utils(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i,rule)|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"typescript","rule":rule,"utils":utils}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"publication","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}

#[inline(never)]
fn check(rules: &[Value], sources: &[&str], fault: Option<&str>) -> Value {
    check_work(&policy(rules), rules, sources, fault, i64::MAX)
}

pub(super) fn check_work(
    p: &CompiledCodeRuleDsl,
    rules: &[Value],
    sources: &[&str],
    fault: Option<&str>,
    allowance: i64,
) -> Value {
    check_work_compiler(p, rules, sources, fault, allowance, super::compile_job)
}

pub(super) fn check_work_compiler(
    p: &CompiledCodeRuleDsl,
    rules: &[Value],
    sources: &[&str],
    fault: Option<&str>,
    allowance: i64,
    compiler: impl FnOnce(&[&matcher::GrammarMatcher], i64) -> Result<super::super::Compiled, String>,
) -> Value {
    let grammars: Vec<_> = p
        .inner
        .clauses
        .iter()
        .map(|clause| {
            let crate::code_rule_dsl::model::ClauseKind::Forbid(
                crate::code_rule_dsl::model::Query::Ast { matchers, .. },
            ) = &clause.kind
            else {
                panic!("AST clause required")
            };
            &matchers[0]
        })
        .collect();
    assert_eq!(grammars.len(), rules.len());
    let docs: Vec<_> = sources
        .iter()
        .map(|s| tape(grammars[0].language, s))
        .collect();
    let mut expected = vec![];
    let mut diagnostics = vec![];
    let mut native_traces = vec![];
    for (file, doc) in docs.iter().enumerate() {
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        for (clause, grammar) in grammars.iter().enumerate() {
            let names = matcher::CaptureAnalysis::default()
                .binds(&grammar.matcher)
                .union(&matcher::CaptureAnalysis::default().counts(&grammar.matcher))
                .cloned()
                .collect::<Vec<_>>();
            let mut provisional = vec![];
            let mut work = allowance as usize;
            let candidates_before = matcher::candidates_tested();
            let (result, calls) = value_probe::run_reached(fault, || {
                matcher::with_file_memo(&tree, || {
                    matcher::run_clause(
                        &tree,
                        grammar,
                        None,
                        doc.transducer_len(),
                        &mut work,
                        |root, env, left| {
                            let captures: Vec<_> = names
                                .iter()
                                .map(|name| {
                                    env.get_labels(name)
                                        .map(|nodes| {
                                            json!(
                                                nodes
                                                    .iter()
                                                    .map(|n| n.node_id() as u32)
                                                    .collect::<Vec<_>>()
                                            )
                                        })
                                        .or_else(|| {
                                            env.get_match(name).map(|n| json!(n.node_id() as u32))
                                        })
                                        .or_else(|| {
                                            env.get_transformed(name).map(|n| {
                                                if let (Some(text),Some(span))=(matcher::exported_text(&env,name),matcher::exported_span(&env,name)) {
                                                    json!({"text":text,"span":[span.start,span.end]})
                                                } else {json!(String::from_utf8_lossy(n).into_owned())}
                                            })
                                        })
                                        .unwrap_or(Value::Null)
                                })
                                .collect();
                            provisional.push(json!([clause, file, root, captures, 0, *left]));
                            Ok::<_, ()>(true)
                        },
                    )
                    .unwrap()
                })
            });
            let prior = provisional.len();
            match result {
                Ok(()) => expected.extend(provisional),
                Err(Stopped::Limit(limit)) => {
                    diagnostics.push(crate::code_rule_dsl::eval::failure(
                        &format!("c{clause}"),
                        "budget-exceeded",
                        limit.message(),
                        Some(&format!("{file}.ts")),
                    ));
                    expected.push(json!([
                        clause,
                        file,
                        null,
                        [],
                        failure::encode(&format!("{limit:?}")).unwrap(),
                        work
                    ]));
                }
                Err(Stopped::Deadline) => panic!("unexpected deadline"),
            }
            native_traces.push(
                json!({"clause":clause,"file":file,"calls":calls,"provisionalMatches":prior,"remaining":work,"candidateVisits":matcher::candidates_tested()-candidates_before}),
            );
        }
    }
    expected.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64(), row[2].as_u64()));
    let generation_start = std::time::Instant::now();
    let compiled = compiler(&grammars, allowance).unwrap();
    let generation_wall_ns = generation_start.elapsed().as_nanos();
    let (((actual, metrics), calls), budget_calls) = super::super::budget_leaf::trace(|| {
        value_probe::run_reached(fault, || {
            exec::with_terminals(
                docs.into_iter().map(|d| (d, Default::default())).collect(),
                compiled.terminals,
                |db, batch, _| {
                    super::super::leaves::register(db, batch).unwrap();
                    let ((actual, metrics), frames) = profiling::frame_counts(|| {
                        let memo_start = super::super::memo_store::counts();
                        let row_start = super::super::row::ROWS.get();
                        let cell_start = super::super::row::CELLS.get();
                        let continuation_start = super::super::continuation_store::traffic();
                        let stages =
                            super::super::spike_stages::run(db, &compiled.relations, &compiled.sql)
                                .unwrap();
                        let start = std::time::Instant::now();
                        let prepare_started = crate::code_rule_dsl::sql::timing::Started::now();
                        let mut stmt = db.prepare(&compiled.sql).unwrap();
                        let prepare = start.elapsed().as_nanos();
                        let prepare_cpu = prepare_started.elapsed().cpu;
                        let trace = ExecutionTrace::start(db);
                        let final_started = crate::code_rule_dsl::sql::timing::Started::now();
                        let actual: Vec<Value> = stmt
                            .query_map([], |r| {
                                let tag = r.get::<_, i64>(4)?;
                                Ok(json!([
                                    r.get::<_, u32>(0)?,
                                    r.get::<_, u32>(1)?,
                                    if tag < 0 {
                                        None
                                    } else {
                                        Some(r.get::<_, u32>(2)?)
                                    },
                                    serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap(),
                                    tag,
                                    r.get::<_, i64>(5)?
                                ]))
                            })
                            .unwrap()
                            .collect::<rusqlite::Result<_>>()
                            .unwrap();
                        let final_elapsed = final_started.elapsed();
                        let final_executions = trace.count();
                        let executions = final_executions + stages.executions;
                        drop(trace);
                        let execute_wall_ns = start.elapsed().as_nanos();
                        let (state_rows,state_peak,state_scopes,scope_peak,continuation_rows,continuation_peak)=db.query_row("SELECT relation_state_rows(),relation_state_peak(),relation_state_scopes(),relation_state_scope_peak(),relation_continuation_rows(),relation_continuation_peak()",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).unwrap();
                        assert_eq!(state_rows, 0, "completed SQL left typed capture snapshots");
                        assert_eq!(
                            state_scopes, 0,
                            "completed SQL left capture invocation scopes"
                        );
                        assert_eq!(
                            continuation_rows, 0,
                            "completed SQL left continuation frames"
                        );
                        let memo = super::super::memo_store::counts()
                            .into_iter()
                            .zip(memo_start)
                            .map(|(end, start)| end - start)
                            .collect::<Vec<_>>();
                        let mut metrics = json!({"memoStorage":{"resets":memo[0],"lookups":memo[1],"hits":memo[2],"publications":memo[3],"integerCellsStored":memo[4]},"typedReturnRows":super::super::row::ROWS.get()-row_start,"typedReturnCells":super::super::row::CELLS.get()-cell_start,"matchingStatements":executions,"prepareWallNs":prepare+u128::from(stages.prepare.wall),"prepareExecuteWallNs":execute_wall_ns+u128::from(stages.prepare.wall)+u128::from(stages.execution.wall),"captureStateRows":state_rows,"captureStatePeak":state_peak,"captureScopes":state_scopes,"captureScopePeak":scope_peak,"continuationRows":continuation_rows,"continuationPeak":continuation_peak,"continuationTraffic":super::super::continuation_store::traffic().into_iter().zip(continuation_start).map(|(end,start)|end-start).collect::<Vec<_>>(),"vmSteps":i64::from(stmt.get_status(rusqlite::StatementStatus::VmStep))+stages.vm,"automaticIndexRows":i64::from(stmt.get_status(rusqlite::StatementStatus::AutoIndex))+stages.automatic_index,"fullscanSteps":i64::from(stmt.get_status(rusqlite::StatementStatus::FullscanStep))+stages.fullscan});
                        if stages.active() {
                            metrics["stageExecution"] = stages.profile();
                            metrics["finalStatement"] = json!({"matchingStatements":final_executions,"vmSteps":stmt.get_status(rusqlite::StatementStatus::VmStep),"automaticIndexRows":stmt.get_status(rusqlite::StatementStatus::AutoIndex),"fullscanSteps":stmt.get_status(rusqlite::StatementStatus::FullscanStep),"prepareCpuNs":prepare_cpu,"prepareWallNs":prepare,"executionCpuNs":final_elapsed.cpu,"executionWallNs":final_elapsed.wall});
                            metrics["prepareCpuNs"] = json!(prepare_cpu + stages.prepare.cpu);
                            metrics["executionCpuNs"] =
                                json!(final_elapsed.cpu + stages.execution.cpu);
                            metrics["executionWallNs"] =
                                json!(final_elapsed.wall + stages.execution.wall);
                        }
                        (actual, metrics)
                    });
                    assert_eq!(frames[4], 0);
                    let expected_statements = compiled.relations["spikeStages"]
                        .as_array()
                        .map_or(1, |s| s.len() + 1);
                    assert_eq!(metrics["matchingStatements"], expected_statements);
                    let explain: Vec<_> = db
                        .prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql))
                        .unwrap()
                        .query_map([], |r| r.get::<_, String>(3))
                        .unwrap()
                        .collect::<rusqlite::Result<_>>()
                        .unwrap();
                    (
                        actual,
                        json!({"execution":metrics,"framePublications":frames[4],"nodeAdvances":frames[11],"nodeFilters":frames[13],"explain":explain}),
                    )
                },
            )
        })
    });
    for trace in &native_traces {
        let reached: Vec<_> = budget_calls
            .iter()
            .filter(|call| {
                call["file"] == trace["file"]
                    && compiled.relations["terminalOwners"]
                        [call["terminal"].as_u64().unwrap() as usize]
                        == trace["clause"]
            })
            .flat_map(|call| call["valueCalls"].as_array().into_iter().flatten().cloned())
            .collect();
        assert_eq!(
            json!(reached),
            trace["calls"],
            "reached calls for clause/file {trace}, allowance={allowance}, rules={rules:?}"
        );
    }
    assert_eq!(
        actual, expected,
        "rules={rules:?}, sources={sources:?}, calls={calls:?}"
    );
    let mut actual_diagnostics: Vec<_> = actual
        .iter()
        .filter(|row| row[4].as_i64().unwrap() < 0)
        .map(|row| {
            failure::diagnostic(
                row[4].as_i64().unwrap(),
                &format!("c{}", row[0]),
                &format!("{}.ts", row[1]),
            )
            .unwrap()
        })
        .collect();
    diagnostics.sort_by_key(Value::to_string);
    actual_diagnostics.sort_by_key(Value::to_string);
    assert_eq!(actual_diagnostics, diagnostics);
    let result = json!({"parity":true,"allowance":allowance,"generationWallNs":generation_wall_ns,"budgetCalls":budget_calls,"output":actual,"nativeTraces":native_traces,"sqlCalls":calls,"metrics":metrics,"sql":compiled.sql,"relations":compiled.relations});
    println!(
        "PUBLICATION_TRACE {}",
        json!({"output":result["output"],"nativeTraces":result["nativeTraces"],"sqlCalls":result["sqlCalls"]})
    );
    result
}
fn value() -> Value {
    json!({"kind":"number","pattern":"$X","value":{"gt":0}})
}
fn total() -> Value {
    json!({"kind":"number","pattern":"$Y"})
}

#[test]
fn publication_discards_failed_pair_and_retains_sibling_files_and_clauses() {
    for sources in [["1;2;3;", "4;5;6;"], ["4;5;6;", "1;2;3;"]] {
        for rules in [[value(), total()], [total(), value()]] {
            let r = check(&rules, &sources, Some("2"));
            assert_eq!(r["output"].as_array().unwrap().len(), 10); // nine matches and one failure
            assert!(!r["sqlCalls"].as_array().unwrap().contains(&json!("3")));
            assert_eq!(r["sqlCalls"].as_array().unwrap().len(), 5);
        }
    }
}
#[test]
fn publication_failure_is_not_boolean_false_or_unknown() {
    for rule in [
        json!({"kind":"number","not":{"value":{"gt":0}}}),
        json!({"kind":"number","any":[{"value":{"gt":0}},{"regex":".*"}]}),
        json!({"kind":"number","all":[{"value":{"gt":0}},{"regex":"^never$"}]}),
        json!({"kind":"arguments","has":{"kind":"number","pattern":"$X","value":{"eq":2}}}),
        json!({"kind":"arguments","count":{"has":{"kind":"number","value":{"gt":0}},"min":0,"max":10,"as":"N"}}),
        json!({"kind":"arguments","has":{"kind":"array","pattern":"$A","has":{"kind":"number","pattern":"$X","value":{"eq":2}},"stopBy":"neighbor"}}),
    ] {
        check(
            &[rule, total()],
            &["f([1,2,3]); g(1,2,3);", "h([4]);"],
            Some("2"),
        );
    }
}
#[test]
fn publication_decisive_branches_keep_failures_unreached() {
    for rule in [
        json!({"kind":"number","any":[{"regex":".*"},{"value":{"gt":0}}]}),
        json!({"kind":"number","all":[{"regex":"^never$"},{"value":{"gt":0}}]}),
        json!({"kind":"arguments","has":{"kind":"number","value":{"eq":1}}}),
    ] {
        check(&[rule, total()], &["f(1,2,3);", "f(1,2,3);"], Some("2"));
    }
}
#[test]
fn publication_stateless_api_discards_the_entire_failed_clause() {
    let p = policy(&[value(), total()]);
    for files in [
        json!([{"path":"a.ts","language":"typescript","source":"1;2;3;"},{"path":"b.ts","language":"typescript","source":"4;5;6;"}]),
        json!([{"path":"b.ts","language":"typescript","source":"4;5;6;"},{"path":"a.ts","language":"typescript","source":"1;2;3;"}]),
    ] {
        let (result, _) = value_probe::run(Some("2"), || {
            p.evaluate(json!({"format":"rerail.repository-snapshot/v1","revision":0,"facts":[],"files":files}).to_string(),"report".into(),Some("native".into())).unwrap()
        });
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["violations"].as_array().unwrap().len(), 6);
        assert!(
            result["violations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["clauseId"] == "c1")
        );
        assert_eq!(
            result["failures"],
            json!([{"clauseId":"c0","code":"budget-exceeded","message":matcher::UtilLimit::Value.message(),"path":"a.ts"}])
        );
    }
}
#[test]
fn publication_scales_across_roots_files_and_clauses() {
    let mut samples = vec![];
    for dimension in ["roots", "early-failure", "files", "clauses"] {
        for size in [4, 16, 64] {
            let source = if dimension == "early-failure" {
                format!("2;{}", "1;".repeat(size))
            } else {
                "1;".repeat(if dimension == "roots" { size } else { 1 })
            };
            let sources = vec![source.as_str(); if dimension == "files" { size } else { 1 }];
            let rules = vec![value(); if dimension == "clauses" { size / 2 } else { 2 }];
            let mut r = check(&rules, &sources, Some("2"));
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_PUBLICATION_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
                    <= 5 * pair[0]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
            );
        }
    }
}

#[test]
fn publication_multiple_failures_are_isolated_and_empty_files_are_empty() {
    let r = check(
        &[value(), value(), total()],
        &["1;2;3;", "2;3;", "", "4;"],
        Some("2"),
    );
    let mut diagnostics: Vec<_> = r["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row[4].as_i64().unwrap() < 0)
        .map(|row| {
            failure::diagnostic(
                row[4].as_i64().unwrap(),
                &format!("c{}", row[0]),
                &format!("{}.ts", row[1]),
            )
            .unwrap()
        })
        .collect();
    diagnostics.sort_by(|a, b| {
        crate::code_rule_dsl::eval::result_order(a)
            .cmp(&crate::code_rule_dsl::eval::result_order(b))
    });
    let expected: Vec<_> = [0,1].into_iter().flat_map(|file|[0,1].into_iter().map(move |clause|json!({"clauseId":format!("c{clause}"),"path":format!("{file}.ts"),"code":"budget-exceeded","message":matcher::UtilLimit::Value.message()}))).collect();
    assert_eq!(diagnostics, expected);
    assert!(failure::diagnostic(-5, "c0", "0.ts").is_err());
    assert_eq!(
        r["output"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row[4].as_i64().unwrap() < 0)
            .count(),
        4
    );
    assert!(
        !r["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[1] == 2)
    );
    assert!(!r["sqlCalls"].as_array().unwrap().contains(&json!("3")));
    assert_eq!(r["sqlCalls"].as_array().unwrap().len(), 8);
    check(&[total()], &["", "1;2;"], None);
    let rules = vec![value(); 33];
    let p = policy(&rules);
    let grammars: Vec<_> = p
        .inner
        .clauses
        .iter()
        .map(|c| {
            let crate::code_rule_dsl::model::ClauseKind::Forbid(
                crate::code_rule_dsl::model::Query::Ast { matchers, .. },
            ) = &c.kind
            else {
                panic!("AST")
            };
            &matchers[0]
        })
        .collect();
    assert!(
        super::compile_job(&grammars, i64::MAX)
            .err()
            .unwrap()
            .contains("1..32")
    );
}

#[test]
fn spike_stages_control_counts_ctas_index_work_and_preserves_native_probes() {
    let rules = [value()];
    let p = policy(&rules);
    let sources = ["f(1,2,3);", "g(4,5);"];
    let ordinary = check_work(&p, &rules, &sources, None, 100);
    assert_eq!(
        ordinary["metrics"]["execution"]["matchingStatements"],
        json!(1)
    );
    assert!(ordinary["metrics"]["execution"]["stageExecution"].is_null());
    for allowance in [0, 1, 100] {
        for fault in [None, Some("2")] {
            let result =
                check_work_compiler(&p, &rules, &sources, fault, allowance, |grammars, work| {
                    let mut compiled = super::compile_job(grammars, work)?;
                    let needle = "FROM node n";
                    if !compiled.sql.contains(needle) {
                        return Err("stage control: root node input not found".into());
                    }
                    compiled.sql = compiled.sql.replacen(
                        needle,
                        "FROM stage_control cut JOIN node n ON n.file=cut.file AND n.id=cut.node",
                        1,
                    );
                    compiled.relations["spikeStages"] = json!([
                        "CREATE TEMP TABLE stage_control AS SELECT file,id AS node FROM node",
                        "CREATE UNIQUE INDEX stage_control_key ON stage_control(file,node)"
                    ]);
                    Ok(compiled)
                });
            let metrics = &result["metrics"]["execution"];
            assert_eq!(metrics["matchingStatements"], json!(3));
            let stages = &metrics["stageExecution"];
            assert_eq!(stages["stageExecutions"], json!(2));
            assert_eq!(stages["statements"].as_array().unwrap().len(), 2);
            let stage_vm = stages["vmSteps"].as_i64().unwrap();
            assert!(stage_vm > 0);
            assert_eq!(
                metrics["vmSteps"].as_i64().unwrap(),
                stage_vm + metrics["finalStatement"]["vmSteps"].as_i64().unwrap()
            );
            assert!(
                metrics["vmSteps"].as_i64().unwrap()
                    > metrics["finalStatement"]["vmSteps"].as_i64().unwrap()
            );
        }
    }
}
