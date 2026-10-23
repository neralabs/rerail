use super::super::{failure, tests::ExecutionTrace};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    matcher::{self, Stopped, value_probe},
    sql::{exec, profiling, tests::differential::tape},
    test_support::CAPABILITIES,
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

fn policy(rules: &[Value]) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i,rule)|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"typescript","rule":rule}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"publication","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}

#[inline(never)]
fn check(rules: &[Value], sources: &[&str], fault: Option<&str>) -> Value {
    let p = policy(rules);
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
    let mut native_traces = vec![];
    for (clause, grammar) in grammars.iter().enumerate() {
        let names = matcher::CaptureAnalysis::default()
            .binds(&grammar.matcher)
            .union(&matcher::CaptureAnalysis::default().counts(&grammar.matcher))
            .cloned()
            .collect::<Vec<_>>();
        for (file, doc) in docs.iter().enumerate() {
            let tree = AstGrep::doc(doc.clone());
            let mut provisional = vec![];
            let (result, calls) = value_probe::run(fault, || {
                matcher::with_file_memo(&tree, || {
                    let mut work = usize::MAX;
                    matcher::run_clause(
                        &tree,
                        grammar,
                        None,
                        doc.transducer_len(),
                        &mut work,
                        |root, env, _| {
                            let captures: Vec<_> = names
                                .iter()
                                .map(|name| {
                                    env.get_match(name)
                                        .map(|n| json!(n.node_id() as u32))
                                        .or_else(|| {
                                            env.get_transformed(name).map(|n| {
                                                json!(String::from_utf8_lossy(n).into_owned())
                                            })
                                        })
                                        .unwrap_or(Value::Null)
                                })
                                .collect();
                            provisional.push(json!([clause, file, root, captures, 0]));
                            Ok::<_, ()>(true)
                        },
                    )
                    .unwrap()
                })
            });
            let prior = provisional.len();
            match result {
                Ok(()) => expected.extend(provisional),
                Err(Stopped::Limit(limit)) => expected.push(json!([
                    clause,
                    file,
                    null,
                    [],
                    failure::encode(&format!("{limit:?}")).unwrap()
                ])),
                Err(Stopped::Deadline) => panic!("unexpected deadline"),
            }
            native_traces.push(
                json!({"clause":clause,"file":file,"calls":calls,"provisionalMatches":prior}),
            );
        }
    }
    let compiled = super::compile_job(&grammars).unwrap();
    let ((actual, metrics), calls) = value_probe::run(fault, || {
        exec::with_terminals(
            docs.into_iter().map(|d| (d, Default::default())).collect(),
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let ((actual, metrics), frames) = profiling::frame_counts(|| {
                    let row_start = super::super::row::ROWS.get();
                    let cell_start = super::super::row::CELLS.get();
                    let start = std::time::Instant::now();
                    let mut stmt = db.prepare(&compiled.sql).unwrap();
                    let prepare = start.elapsed().as_nanos();
                    let trace = ExecutionTrace::start(db);
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
                                tag
                            ]))
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<_>>()
                        .unwrap();
                    let executions = trace.count();
                    drop(trace);
                    let metrics = json!({"typedReturnRows":super::super::row::ROWS.get()-row_start,"typedReturnCells":super::super::row::CELLS.get()-cell_start,"matchingStatements":executions,"prepareWallNs":prepare,"prepareExecuteWallNs":start.elapsed().as_nanos(),"vmSteps":stmt.get_status(rusqlite::StatementStatus::VmStep),"automaticIndexRows":stmt.get_status(rusqlite::StatementStatus::AutoIndex),"fullscanSteps":stmt.get_status(rusqlite::StatementStatus::FullscanStep)});
                    (actual, metrics)
                });
                assert_eq!(frames[4], 0);
                assert_eq!(metrics["matchingStatements"], 1);
                let explain: Vec<_> = db
                    .prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql))
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(3))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                (
                    actual,
                    json!({"execution":metrics,"framePublications":frames[4],"explain":explain}),
                )
            },
        )
    });
    assert_eq!(
        actual, expected,
        "rules={rules:?}, sources={sources:?}, calls={calls:?}"
    );
    let result = json!({"parity":true,"output":actual,"nativeTraces":native_traces,"sqlCalls":calls,"metrics":metrics,"sql":compiled.sql,"relations":compiled.relations});
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
        super::compile_job(&grammars)
            .err()
            .unwrap()
            .contains("1..32")
    );
}
