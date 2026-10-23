use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn fixture() -> (Value, Value) {
    (
        json!({"kind":"number","all":[{"matches":"outer"},{"not":{"not":{"matches":"outer"}}}]}),
        json!({"inner":{"value":{"gt":0}},"outer":{"all":[{"matches":"inner"},{"not":{"not":{"matches":"inner"}}}]}}),
    )
}

#[test]
fn nested_memo_native_boundary_and_admission() {
    use crate::code_rule_dsl::{
        matcher::{self, value_probe},
        sql::tests::differential::tape,
    };
    use ast_grep_core::AstGrep;
    let (rule, utils) = fixture();
    let rules = [rule];
    let p = policy_utils(&rules, utils);
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    let grammar = &matchers[0];
    for allowance in [9, 10] {
        let doc = tape(grammar.language, "1;");
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let mut work = allowance;
        let mut matches = 0;
        let (result, calls) = value_probe::run_reached(None, || {
            matcher::with_file_memo(&tree, || {
                matcher::run_clause(
                    &tree,
                    grammar,
                    None,
                    doc.transducer_len(),
                    &mut work,
                    |_, _, _| {
                        matches += 1;
                        Ok::<_, ()>(true)
                    },
                )
                .unwrap()
            })
        });
        println!(
            "NESTED_NATIVE_GATE {}",
            json!({"allowance":allowance,"remaining":work,"matches":matches,"result":match &result {Ok(())=>"ok".into(),Err(matcher::Stopped::Limit(limit))=>format!("{limit:?}"),Err(matcher::Stopped::Deadline)=>"deadline".into()},"calls":calls})
        );
        assert_eq!(result.is_ok(), allowance == 10);
        assert_eq!(work, 0);
        assert_eq!(calls, vec!["1"]);
    }
    for allowance in 0..=11 {
        let r = check_work(&p, &rules, &["1;", "2;"], None, allowance);
        if allowance == 9 {
            assert_eq!(r["output"][0][4], -2);
        }
        if allowance == 10 {
            assert_eq!(r["output"][0][4], 0);
            assert_eq!(r["output"][0][5], 0);
            assert_eq!(r["sqlCalls"].as_array().unwrap().len(), 2);
        }
    }
}

fn check(rule: Value, utils: Value, sources: &[&str], maximum: i64) -> Vec<Value> {
    let rules = [rule, json!({"kind":"number","pattern":"$Y"})];
    let p = policy_utils(&rules, utils);
    (0..=maximum)
        .map(|allowance| check_work(&p, &rules, sources, None, allowance))
        .collect()
}

#[test]
fn nested_memo_inner_answers_survive_false_unknown_and_outer_replay() {
    for (kind, source) in [("number", "1;2;"), ("identifier", "x;y;")] {
        for value in [json!({"gt":0}), json!({"eq":99})] {
            let utils = json!({"inner":{"value":value},"outer":{"all":[{"matches":"inner"},{"not":{"not":{"matches":"inner"}}}]}});
            for terms in [
                json!({"any":[{"matches":"outer"},{"matches":"inner"}]}),
                json!({"all":[{"matches":"outer"},{"matches":"inner"},{"not":{"not":{"matches":"outer"}}}]}),
                json!({"any":[{"matches":"outer"},{"not":{"not":{"matches":"outer"}}},{"matches":"inner"}]}),
            ] {
                check(
                    json!({"kind":kind,"all":[terms.clone()]}),
                    utils.clone(),
                    &[source, source],
                    14,
                );
                check(
                    json!({"kind":kind,"not":terms}),
                    utils.clone(),
                    &[source, source],
                    14,
                );
            }
        }
    }
    let result = check(
        json!({"kind":"number","not":{"any":[{"matches":"outer"},{"matches":"inner"}]}}),
        json!({"inner":{"value":{"eq":99}},"outer":{"matches":"inner"}}),
        &["1;", "2;"],
        5,
    );
    assert_eq!(result[4]["output"][0][4], -2);
    assert_eq!(result[5]["output"][0][5], 0);
    assert_eq!(result[5]["sqlCalls"].as_array().unwrap().len(), 2);
}

#[test]
fn nested_memo_preserves_keys_exports_and_utility_identity() {
    for (rule, utils, source) in [
        (
            json!({"kind":"number","all":[{"not":{"not":{"matches":"outer"}}},{"matches":"inner"},{"matches":"outer"}]}),
            json!({"inner":{"pattern":"$X","value":{"gt":0}},"outer":{"matches":"inner"}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","all":[{"matches":"outer"},{"pattern":"$X"},{"not":{"not":{"matches":"outer"}}},{"matches":"inner"}]}),
            json!({"inner":{"value":{"gt":0}},"outer":{"matches":"inner"}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","all":[{"matches":"outer"},{"not":{"not":{"matches":"outer"}}},{"matches":"inner"}]}),
            json!({"inner":{"value":{"gt":0}},"outer":{"all":[{"pattern":"$X"},{"matches":"inner"}]}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","any":[{"all":[{"matches":"outer"},{"regex":"^never$"}]},{"matches":"inner"},{"matches":"outer"}]}),
            json!({"inner":{"pattern":"$X","value":{"gt":0}},"outer":{"matches":"inner"}}),
            "1;2;",
        ),
        (
            json!({"kind":"identifier","all":[{"matches":"outer"},{"not":{"not":{"matches":"inner"}}},{"matches":"outer"}]}),
            json!({"inner":{"pattern":"$X","value":{"gt":0}},"outer":{"matches":"inner"}}),
            "x;y;",
        ),
        (
            json!({"kind":"arguments","all":[{"not":{"not":{"matches":"outer"}}},{"matches":"inner"},{"matches":"outer"}]}),
            json!({"inner":{"count":{"has":{"kind":"number","value":{"gt":0}},"min":1,"as":"N"}},"outer":{"matches":"inner"}}),
            "f(1,2);",
        ),
        (
            json!({"kind":"number","all":[{"matches":"left"},{"matches":"right"},{"matches":"inner"},{"not":{"not":{"matches":"left"}}}]}),
            json!({"inner":{"value":{"gt":0}},"left":{"matches":"inner"},"right":{"matches":"inner"}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","all":[{"matches":"outer"},{"not":{"not":{"matches":"outer"}}}]}),
            json!({"a":{"value":{"gt":0}},"b":{"value":{"gt":0}},"outer":{"all":[{"matches":"a"},{"matches":"b"},{"matches":"a"},{"matches":"b"}]}}),
            "1;2;",
        ),
    ] {
        check(rule, utils, &[source, source], 20);
    }
}

#[test]
fn nested_memo_guards_body_failures_and_optional_inner_calls() {
    for (terms, body) in [
        (
            json!({"all":[{"matches":"outer"},{"not":{"not":{"matches":"outer"}}}]}),
            json!({"matches":"inner"}),
        ),
        (
            json!({"all":[{"matches":"outer"},{"matches":"inner"}]}),
            json!({"any":[{"regex":".*"},{"matches":"inner"}]}),
        ),
        (
            json!({"any":[{"matches":"outer"},{"matches":"inner"}]}),
            json!({"all":[{"regex":"^never$"},{"matches":"inner"}]}),
        ),
        (
            json!({"any":[{"all":[{"regex":"^never$"},{"matches":"outer"}]},{"matches":"outer"},{"matches":"inner"}]}),
            json!({"matches":"inner"}),
        ),
        (
            json!({"all":[{"matches":"outer"},{"any":[{"regex":".*"},{"all":[{"matches":"outer"},{"matches":"inner"}]}]}]}),
            json!({"matches":"inner"}),
        ),
        (
            json!({"any":[{"regex":".*"},{"matches":"outer"},{"matches":"inner"}]}),
            json!({"matches":"inner"}),
        ),
    ] {
        let rule = json!({"kind":"number","all":[terms]});
        let rules = [rule.clone(), rule];
        let p = policy_utils(&rules, json!({"inner":{"value":{"gt":0}},"outer":body}));
        for fault in [None, Some("2")] {
            for work in [0, 1, 2, 3, 4, 5, 6, 10, 100] {
                check_work(&p, &rules, &["1;2;3;", "4;5;6;"], fault, work);
            }
        }
    }
}

#[test]
fn nested_memo_profile() {
    let mut samples = vec![];
    for dimension in ["repeats", "inner-repeats", "roots", "depth", "skipped"] {
        for size in if dimension == "depth" {
            [1, 3, 7]
        } else {
            [2, 4, 8]
        } {
            let mut utils = serde_json::Map::new();
            utils.insert(
                "inner".into(),
                json!({"value":if dimension=="skipped" {json!({"eq":99})}else{json!({"gt":0})}}),
            );
            let outer = if dimension == "depth" {
                let mut name = "inner".to_string();
                for level in 0..size {
                    let next = format!("u{level}");
                    utils.insert(next.clone(), json!({"matches":name}));
                    name = next;
                }
                name
            } else {
                let calls = vec![
                    json!({"matches":"inner"});
                    if dimension == "inner-repeats" {
                        size
                    } else {
                        2
                    }
                ];
                utils.insert("outer".into(), json!({"all":calls}));
                "outer".into()
            };
            let mut calls = vec![
                json!({"not":{"not":{"matches":outer}}});
                if dimension == "repeats" || dimension == "skipped" {
                    size
                } else {
                    2
                }
            ];
            if dimension == "skipped" {
                calls[0] = json!({"all":[{"regex":"^never$"},{"matches":outer}]});
            }
            let rule = if dimension == "skipped" {
                json!({"kind":"number","any":calls})
            } else {
                json!({"kind":"number","all":calls})
            };
            let roots = if dimension == "roots" { size } else { 1 };
            let source = "1;".repeat(roots);
            let rules = [rule];
            let p = policy_utils(&rules, json!(utils));
            let mut r = check_work(&p, &rules, &[&source, &source], None, 10000);
            assert_eq!(r["sqlCalls"].as_array().unwrap().len(), 2 * roots);
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_NESTED_MEMO_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}

#[test]
fn nested_memo_transitive_diamond_replays_all_levels() {
    let r = check(
        json!({"kind":"number","all":[{"matches":"outer"},{"matches":"middle"},{"matches":"inner"},{"not":{"not":{"matches":"outer"}}}]}),
        json!({"inner":{"value":{"gt":0}},"middle":{"all":[{"matches":"inner"},{"matches":"inner"}]},"outer":{"all":[{"matches":"middle"},{"matches":"inner"},{"matches":"middle"}]}}),
        &["1;", "2;"],
        34,
    );
    assert_eq!(r[32]["output"][0][4], -2);
    assert_eq!(r[33]["output"][0][4], 0);
    assert_eq!(r[33]["output"][0][5], 0);
    assert_eq!(r[33]["sqlCalls"].as_array().unwrap().len(), 2);
}

#[test]
fn nested_memo_bounds_answer_storage_before_execution() {
    let mut utils = serde_json::Map::new();
    utils.insert("inner".into(), json!({"value":{"gt":0}}));
    let mut name = "inner".to_string();
    for level in 0..8 {
        let next = format!("u{level}");
        utils.insert(next.clone(), json!({"matches":name}));
        name = next;
    }
    let p = policy_utils(
        &[json!({"kind":"number","all":[{"matches":name},{"matches":name}]})],
        json!(utils),
    );
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    assert!(
        super::compile_job(&[&matchers[0]], 100)
            .err()
            .unwrap()
            .contains("eight key versions")
    );
}
