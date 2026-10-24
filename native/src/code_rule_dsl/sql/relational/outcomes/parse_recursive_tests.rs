//! Cross-document utility cycles exercise SQL control, fresh bindings and export rollback.
use super::publication_tests::check_work;
use serde_json::{Value, json};

fn check(rule: Value, utils: Value, sources: &[&str]) {
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    for allowance in [0, 1, 2, 3, 4, 5, 8, 12, 20, 32, 64, 128, 1000, 10000] {
        check_work(
            &policy,
            std::slice::from_ref(&rule),
            sources,
            None,
            allowance,
        );
    }
}
fn wrapped(payload: &str, depth: usize) -> String {
    let mut annotation = payload.to_owned();
    for _ in 0..depth {
        annotation = format!("list[{}]", serde_json::to_string(&annotation).unwrap());
    }
    format!("a: {}\n", serde_json::to_string(&annotation).unwrap())
}

#[test]
fn recursive_annotation_exports_star_resolution_through_every_literal() {
    let deep = format!("from package import *\n{}", wrapped("Missing", 6));

    let utils = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","resolves":{"unresolved":"star"},"capture":"NAME"},{"matches":"walk"}]}}}});
    check(
        json!({"kind":"string","matches":"walk"}),
        utils,
        &[
            &deep,
            "from package import *\na: 'list[\"Missing\"]'\nb: 'tuple[\"Known\",\"Other\"]'\nc: 'list[\"list[\\\"Deep\\\"]\"]'\n",
            "a: 'list[\"Missing\"]'\n",
        ],
    );
}
#[test]
fn recursive_annotation_base_keeps_multiple_capture_ranges_and_sibling_order() {
    let deep = wrapped("x is 1", 6);

    let utils = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"comparison_operator","capture":"C","has":{"stopBy":"neighbor","regex":"^is$","capture":"O"}},{"matches":"walk"}]}}}});
    check(
        json!({"kind":"string","matches":"walk"}),
        utils,
        &[
            &deep,
            "a: 'tuple[\"x is 1\",\"y is 2\"]'\nb: 'list[\"list[\\\"x is 3\\\"]\"]'\nc: 'list[\"x == 1\"]'\n",
        ],
    );
}
#[test]
fn recursive_annotation_syntax_errors_export_enclosing_nested_string() {
    let deep = wrapped("a b", 6);

    let kinds = json!({"any":[{"kind":"string"},{"kind":"concatenated_string"}]});
    let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"all":[kinds.clone(),{"parse":{"grammar":"annotation","kind":"ERROR"}}],"capture":"C"},{"all":[kinds,{"matches":"walk"}]}]}}}});
    check(
        json!({"kind":"string","matches":"walk"}),
        utils,
        &[
            &deep,
            "a: 'list[\"a b\"]'\nb: 'tuple[\"int\",\"a b\"]'\nc: 'list[\"list[\\\"a b\\\"]\"]'\nd: 'list[\"int\"]'\n",
        ],
    );
}
fn mutual(base: Value) -> Value {
    json!({
        "within":{"parse":{"grammar":"annotation","has":{"stopBy":"end","all":[{"any":[{"kind":"string"},{"kind":"concatenated_string"}]},{"matches":"error"}]}}},
        "error":{"any":[base,{"matches":"within"}]}
    })
}
#[test]
fn mutual_annotation_semantic_error_cycle_uses_distinct_parsed_documents() {
    let deep = wrapped("await z", 6);

    check(
        json!({"kind":"string","matches":"error"}),
        mutual(json!({"parse":{"grammar":"annotation","has":{"kind":"ERROR","capture":"E"}}})),
        &[
            &deep,
            "a: 'list[\"await x\"]'\nb: 'tuple[\"int\",\"await y\"]'\nc: 'list[\"list[\\\"await z\\\"]\"]'\nd: 'int'\n",
        ],
    );
}
#[test]
fn mutual_annotation_await_cycle_keeps_parent_queries_local_to_subdocument() {
    let deep = wrapped("await z", 6);

    check(
        json!({"kind":"string","matches":"error"}),
        mutual(
            json!({"parse":{"grammar":"annotation","has":{"stopBy":"end","kind":"await","not":{"inside":{"stopBy":"end","kind":"lambda"}},"capture":"A"}}}),
        ),
        &[
            &deep,
            "a: 'list[\"await x\"]'\nb: 'list[\"lambda: await x\"]'\nc: 'list[\"list[\\\"await z\\\"]\"]'\nd: 'int'\n",
        ],
    );
}
#[test]
fn parsed_recursive_unknown_capture_does_not_publish_or_select_later_sibling() {
    let utils = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","capture":"C","value":{"eq":1}},{"matches":"walk"}]}}}});
    check(
        json!({"kind":"string","matches":"walk"}),
        utils,
        &["a: 'tuple[missing,\"1\"]'\nb: 'list[\"missing\"]'\n"],
    );
}
#[test]
fn repeated_parsed_recursive_calls_preserve_transient_entry_and_probe_work() {
    let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","regex":"^int$","capture":"T"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}});
    check(
        json!({"kind":"string","parse":{"grammar":"annotation","has":{"kind":"string","all":[{"matches":"walk"},{"matches":"walk"}],"stopBy":"end"}}}),
        utils,
        &["a: 'list[\"list[int]\"]'\nb: 'list[\"list[str]\"]'\n"],
    );
}

#[test]
fn repeated_host_utility_entries_replay_mapped_capture_deltas_and_exclude_parse_fills() {
    let source = wrapped("int", 4);
    let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","regex":"^int$","capture":"T"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}});
    check(
        json!({"kind":"string","all":[{"matches":"walk"},{"matches":"walk"},{"matches":"walk"}]}),
        utils,
        &[&source],
    );
}

#[test]
fn parsed_recursive_snapshots_are_released_after_success_unknown_and_work_failure() {
    use crate::code_rule_dsl::{
        sql::{exec, tests::differential::tape},
        test_support::grammar_matchers,
    };
    use rusqlite::StatementStatus;
    for unknown in [false, true] {
        let base = if unknown {
            json!({"kind":"identifier","capture":"T","value":{"eq":1}})
        } else {
            json!({"kind":"identifier","regex":"^int$","capture":"T"})
        };
        let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[base,{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}});
        let rule = json!({"kind":"string","matches":"walk"});
        let policy = super::parse_tests::policy("python", rule, utils);
        let grammar = &grammar_matchers(&policy)[0];
        for allowance in [1, 24, 10000] {
            let compiled = super::compile_job(&[grammar], allowance).unwrap();
            assert!(compiled.sql.len() <= 256000);
            let sources = [wrapped("int", 1), wrapped("int", 3), wrapped("int", 6)];
            let docs = sources
                .iter()
                .map(|source| (tape(grammar.language, source), Default::default()))
                .collect();
            exec::with_terminals(docs, compiled.terminals, |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                for run in 0..3 {
                    let mut statement = db.prepare(&compiled.sql).unwrap();
                    let trace = super::super::tests::ExecutionTrace::start(db);
                    let rows = statement
                        .query_map([], |row| row.get::<_, i64>(4))
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    assert_eq!(trace.count(), 1);
                    drop(trace);
                    let vm = statement.get_status(StatementStatus::VmStep);
                    drop(statement);
                    assert_eq!(
                        db.query_row::<i64, _, _>("SELECT relation_state_rows()", [], |r| r.get(0))
                            .unwrap(),
                        0,
                        "unknown={unknown}, allowance={allowance}, run={run}"
                    );
                    assert_eq!(
                        db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r
                            .get(0))
                            .unwrap(),
                        0
                    );
                    eprintln!(
                        "PARSED_RECURSIVE_STORAGE {}",
                        json!({"unknown":unknown,"allowance":allowance,"run":run,"sqlBytes":compiled.sql.len(),"vmSteps":vm,"rows":rows})
                    );
                }
            });
        }
    }
}

#[test]
fn parsed_recursive_control_refuses_capture_dependent_unknown_continuations_and_host_hooks() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    {
        let body = json!({"parse":{"grammar":"annotation","has":{"stopBy":"end","all":[{"kind":"string"},{"matches":"walk"},{"capture":"LATER"}]}}});
        let policy = super::parse_tests::policy(
            "python",
            json!({"kind":"string","matches":"walk"}),
            json!({"walk":body}),
        );
        let grammar = &grammar_matchers(&policy)[0];
        assert!(super::compile_job(&[grammar], 10000).is_err());
    }
    let policy = json!({"format":"rerail.policy/v1","id":"unsupported-host","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{"language":"python","rule":{"kind":"string","parse":{"grammar":"annotation","host":{"kind":"string"}}}}},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]});
    assert!(
        crate::code_rule_dsl::CodeRuleDslRuntime::compile(
            policy.to_string(),
            "json".into(),
            crate::code_rule_dsl::test_support::CAPABILITIES.into()
        )
        .is_err()
    );
}

#[test]
fn catalog_forward_annotation_semantic_clause_has_positive_and_negative_witnesses() {
    use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../packs/python/rules/ruff-f722-forward-annotation-syntax-error.json"
    ));
    let document: Value = serde_json::from_str(text).unwrap();
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../packs/python/upstream/ruff/fixtures/rerail/sql_compiler/F722_semantic.py"
    ));
    // The catalog contract compiles one grammar at a time. Keep that same
    // statement scope rather than concatenate three independent SQL programs.
    for (index, clause) in document["clauses"].as_array().unwrap().iter().enumerate() {
        let rule = clause["forbid"]["ast"]["rule"].clone();
        let mut one = document.clone();
        one["clauses"] = json!([clause]);
        let policy =
            CodeRuleDslRuntime::compile(one.to_string(), "json".into(), CAPABILITIES.into())
                .unwrap();
        let result = check_work(
            &policy,
            std::slice::from_ref(&rule),
            &[source],
            None,
            100000,
        );
        if index == 1 {
            assert_eq!(
                result["output"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| !row[2].is_null())
                    .count(),
                2,
                "semantic clause must match both positive annotations and neither negative annotation"
            );
        }
    }
}

#[test]
fn negative_recursive_captures_have_no_entry_outputs_and_bounded_live_snapshots() {
    use crate::code_rule_dsl::{
        sql::{exec, tests::differential::tape},
        test_support::grammar_matchers,
    };
    use rusqlite::StatementStatus;
    let rule = json!({"kind":"string","matches":"walk"});
    let utils = json!({"walk":{"parse":{"grammar":"annotation","not":{"has":{"stopBy":"end","any":[{"kind":"identifier","regex":"^int$","capture":"HIDDEN"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}}});
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    let grammar = &grammar_matchers(&policy)[0];
    let mut measures = vec![];
    let mut prior_sql = None;
    for count in [16, 96] {
        let elements = std::iter::repeat_n("\"int\"", count)
            .collect::<Vec<_>>()
            .join(",");
        let inner = format!("tuple[{elements}]");
        let annotation = format!("list[{}]", serde_json::to_string(&inner).unwrap());
        let source = format!("a: {}\n", serde_json::to_string(&annotation).unwrap());
        check_work(
            &policy,
            std::slice::from_ref(&rule),
            &[&source],
            None,
            100000,
        );
        let compiled = super::compile_job(&[grammar], 100000).unwrap();
        if let Some(sql) = &prior_sql {
            assert_eq!(&compiled.sql, sql);
        } else {
            prior_sql = Some(compiled.sql.clone());
        }
        let metrics = exec::with_terminals(
            vec![(tape(grammar.language, &source), Default::default())],
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let mut stmt = db.prepare(&compiled.sql).unwrap();
                let captures = stmt
                    .query_map([], |row| row.get::<_, String>(3))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                assert!(
                    captures
                        .iter()
                        .all(|captures| serde_json::from_str::<Value>(captures).unwrap()
                            == json!([])),
                    "negative probes must not publish HIDDEN"
                );
                let vm = stmt.get_status(StatementStatus::VmStep);
                drop(stmt);
                let peak = db
                    .query_row::<i64, _, _>("SELECT relation_state_peak()", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(
                    db.query_row::<i64, _, _>("SELECT relation_state_rows()", [], |row| row.get(0))
                        .unwrap(),
                    0
                );
                (peak, vm)
            },
        );
        measures.push(metrics);
    }
    assert!(
        measures[1].0 <= measures[0].0 + 2,
        "failed-candidate count must not grow live snapshot storage: {measures:?}"
    );
    assert!(
        measures[1].1 <= measures[0].1 * 8,
        "sixfold branching should remain linear in VM work: {measures:?}"
    );
    eprintln!(
        "PARSED_RECURSIVE_BRANCHING {}",
        json!({"small":measures[0],"large":measures[1]})
    );
}

#[test]
fn recursive_parse_setup_preserves_nonliteral_utility_work_and_literal_probes() {
    let rule = json!({"any":[{"regex":"^never$"},{"matches":"entry"}]});
    let utils = json!({
        "entry":{"matches":"walk"},
        "walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"integer","value":{"gt":0},"capture":"T"},{"matches":"walk"}]}}}
    });
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    for width in [0, 4, 32] {
        let mut source = (0..width)
            .map(|index| format!("v{index}={index}\n"))
            .collect::<String>();
        source.push_str(&wrapped("1", 3));
        source.push_str("plain='list[int]'\nunknown: f'{missing}'\n");
        for allowance in [0, 1, 2, 3, 8, 32, 128, 1000, 10000] {
            for fault in [None, Some("1")] {
                let result = check_work(
                    &policy,
                    std::slice::from_ref(&rule),
                    &[&source],
                    fault,
                    allowance,
                );
                if allowance == 10000 && fault.is_none() {
                    // Matching callbacks precede later failed roots. Derive the
                    // total independently from the native end-of-file trace and
                    // pin the failure boundary after the final unmatched root.
                    for trace in result["nativeTraces"].as_array().unwrap() {
                        let spent = allowance - trace["remaining"].as_i64().unwrap();
                        assert!(spent > 0);
                        for boundary in [spent - 1, spent] {
                            check_work(
                                &policy,
                                std::slice::from_ref(&rule),
                                &[&source],
                                None,
                                boundary,
                            );
                        }
                    }
                    eprintln!(
                        "PARSED_SETUP_WIDTH {}",
                        json!({"width":width,"metrics":result["metrics"],"sqlBytes":result["sql"].as_str().unwrap().len()})
                    );
                }
            }
        }
    }
}

#[test]
fn rejected_parse_utility_memo_replays_keep_every_nonliteral_entry_charge() {
    let rule = json!({"any":[{"regex":"^never$"},{"matches":"walk"},{"matches":"walk"},{"matches":"walk"}]});
    let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"integer","value":{"gt":0},"capture":"T"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}});
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    assert!(
        crate::code_rule_dsl::test_support::grammar_matchers(&policy)[0]
            .anchors
            .root_kinds()
            .is_none(),
        "the regex arm must keep nonliteral roots in the native candidate stream"
    );
    let source = "x=0\ny=1\n";
    for allowance in [0, 1, 2, 3, 4, 8, 16, 32, 128, 10000] {
        let result = check_work(
            &policy,
            std::slice::from_ref(&rule),
            &[source],
            None,
            allowance,
        );
        if allowance == 10000 {
            let spent = allowance - result["nativeTraces"][0]["remaining"].as_i64().unwrap();
            assert!(spent > 0, "fixture must reach rejected utility entries");
            for boundary in [spent - 1, spent] {
                check_work(
                    &policy,
                    std::slice::from_ref(&rule),
                    &[source],
                    None,
                    boundary,
                );
            }
        }
    }
}

#[test]
fn rejected_atomic_parse_never_enters_a_callback_first_recursive_child() {
    let rule = json!({"kind":"string","parse":{"grammar":"annotation","all":[{"value":{"eq":0}},{"has":{"stopBy":"end","kind":"string","matches":"walk"}}]}});
    let utils = json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"integer","value":{"gt":0},"capture":"T"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}});
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    for allowance in [0, 1, 2, 8, 64, 1000] {
        for fault in [None, Some("list[\"1\"]")] {
            check_work(
                &policy,
                std::slice::from_ref(&rule),
                &["plain='text'\na: 'list[\"1\"]'\nb: f'{missing}'\n"],
                fault,
                allowance,
            );
        }
    }
}
