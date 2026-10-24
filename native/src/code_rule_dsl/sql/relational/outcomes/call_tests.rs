//! Parameter frames are transactions with independent input and output names.
use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::json;

#[test]
fn parameter_frames_unify_duplicate_outputs_and_keep_private_captures_private() {
    for params in [json!(["P", "Q"]), json!(["Q", "P"])] {
        let rules = [json!({"kind":"call_expression","matches":{"pair":{"P":"$X","Q":"$X"}}})];
        let p = policy_utils(
            &rules,
            json!({"pair":{"params":params,"rule":{"pattern":"$P($Q)"}}}),
        );
        let r = check_work(&p, &rules, &["f(g); h(h);"], None, i64::MAX);
        assert_eq!(r["output"].as_array().unwrap().len(), 1);
        let grammar = &grammar_matchers(&p)[0];
        let plan = super::compile_job(&[grammar], i64::MAX).unwrap();
        let names: Vec<_> = plan.relations["captures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].clone())
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.as_str().is_some_and(|s| s.starts_with("@call")))
        );
    }
}

#[test]
fn parameter_frames_import_inputs_and_rollback_false_and_unknown() {
    let source = ["f(a); f(u); g(a);"];
    let utils = json!({"pick":{"params":["P"],"rule":{"pattern":"f($P)","testUnknown":"^f\\(u"}}});
    for rule in [
        json!({"kind":"call_expression","all":[{"pattern":"f($X)"},{"matches":{"pick":{"P":"$X"}}}]}),
        json!({"kind":"call_expression","any":[{"all":[{"matches":{"pick":{"P":"$X"}}},{"regex":"^never$"}]},{"pattern":"g($Y)"}]}),
        json!({"kind":"call_expression","all":[{"not":{"matches":{"pick":{"P":"$X"}}}},{"pattern":"$Y"}]}),
    ] {
        let rules = [rule];
        let p = policy_utils(&rules, utils.clone());
        for work in [0, 1, 2, 3, 8, 64] {
            check_work(&p, &rules, &source, None, work);
        }
    }
}

#[test]
fn parameter_call_sites_preserve_entry_work_and_body_lowering_size() {
    let utils = json!({"leaf":{"params":["P"],"rule":{"pattern":"$P","value":{"gt":0}}}});
    let mut samples = Vec::new();
    for size in [2, 8, 16] {
        let rules =
            [json!({"kind":"number","all":vec![json!({"matches":{"leaf":{"P":"$X"}}});size]})];
        let p = policy_utils(&rules, utils.clone());
        for work in [0, 1, 2, size as i64, size as i64 * 2, 128] {
            check_work(&p, &rules, &["1; 2;"], None, work);
        }
        let grammar = &grammar_matchers(&p)[0];
        let plan = super::compile_job(&[grammar], 128).unwrap();
        assert!(plan.sql.len() < 256_000, "{}", plan.sql.len());
        // The whole-node capture is a SQL bind; the value terminal is shared.
        assert_eq!(plan.terminals.len(), 1);
        let r = check_work(&p, &rules, &["1;"], None, 128);
        samples.push((
            plan.sql.len(),
            r["metrics"]["execution"]["vmSteps"].as_u64().unwrap(),
        ));
    }
    for pair in samples.windows(2) {
        assert!(pair[1].0 <= 5 * pair[0].0);
        assert!(pair[1].1 <= 5 * pair[0].1);
    }
}

#[test]
fn nested_parameter_frames_and_ordinary_utilities_keep_namespaces_separate() {
    let rules = [json!({"kind":"call_expression","all":[
        {"matches":"bind"},
        {"matches":{"outer":{"P":"$X"}}}
    ]})];
    let utils = json!({
        "bind":{"pattern":"f($X)"},
        "outer":{"params":["P"],"rule":{"all":[
            {"matches":"bind"},
            {"matches":{"inner":{"Q":"$P"}}}
        ]}},
        "inner":{"params":["Q"],"rule":{"pattern":"f($Q)"}}
    });
    let p = policy_utils(&rules, utils);
    for work in [0, 1, 2, 3, 4, 8, 64] {
        check_work(&p, &rules, &["f(a); f(b); g(a);"], None, work);
    }
}
#[test]
fn parameter_frames_share_native_utility_answers_with_equal_logical_environments() {
    let utils = json!({
        "leaf":{"pattern":"$P","value":{"gt":0}},
        "frame":{"params":["P"],"rule":{"matches":"leaf"}},
        "nested":{"params":["P"],"rule":{"matches":{"frame":{"P":"$P"}}}}
    });
    let direct = json!({"matches":"leaf"});
    let framed = json!({"matches":{"frame":{"P":"$P"}}});
    let nested = json!({"matches":{"nested":{"P":"$P"}}});
    for terms in [
        vec![direct.clone(), framed.clone()],
        vec![framed.clone(), direct.clone()],
        vec![direct.clone(), nested.clone(), framed.clone()],
        vec![nested, framed, direct],
    ] {
        for extra_caller_capture in [false, true] {
            let mut prefix = vec![json!({"pattern":"$P"})];
            if extra_caller_capture {
                prefix.push(json!({"pattern":"$UNRELATED"}));
            }
            prefix.push(json!({"all":terms.clone()}));
            let rules = [json!({"kind":"number","all":prefix})];
            let p = policy_utils(&rules, utils.clone());
            for work in [0, 1, 2, 3, 4, 5, 8, 64] {
                check_work(&p, &rules, &["1; 2;"], None, work);
            }
        }
    }
}

#[test]
fn parameter_frames_remap_cached_exports_back_to_each_callers_name() {
    let utils = json!({
        "leaf":{"pattern":"$P","value":{"gt":0}},
        "frame":{"params":["P"],"rule":{"matches":"leaf"}}
    });
    for terms in [
        json!([{ "not":{"not":{"matches":"leaf"}}},{"matches":{"frame":{"P":"$X"}}}]),
        json!([{ "not":{"not":{"matches":{"frame":{"P":"$X"}}}}},{"matches":"leaf"}]),
    ] {
        let rules = [json!({"kind":"number","all":terms})];
        let p = policy_utils(&rules, utils.clone());
        for work in [0, 1, 2, 3, 4, 5, 8, 64] {
            check_work(&p, &rules, &["1; 2;"], None, work);
        }
    }
}
#[test]
fn parameter_frames_share_answers_under_the_same_native_following_context() {
    let utils = json!({
        "leaf":{"value":{"gt":0}},
        "project":{"params":["P"],"rule":{"matches":"leaf"}},
        "binding":{"resolves":{"binding":{"value":{"matches":{"project":{"P":"$P"}}}}}},
        "frame":{"params":["P"],"rule":{"matches":"binding"}}
    });
    let direct = json!({"matches":"binding"});
    let framed = json!({"matches":{"frame":{"P":"$P"}}});
    for terms in [vec![direct.clone(), framed.clone()], vec![framed, direct]] {
        // The extra capture makes the enclosing binding utility miss across frames.
        // Its parameterized value child sees the same projected P and FOLLOWING pair.
        // Namespace-specific target columns used to split that one native memo key,
        // evaluating the value leaf twice instead of replaying its completed answer.
        let rules = [json!({"kind":"identifier","regex":"^a$","all":[
            {"pattern":"$P"},{"pattern":"$UNRELATED"},{"all":terms}
        ]})];
        let p = crate::code_rule_dsl::CodeRuleDslRuntime::compile(
            json!({"format":"rerail.policy/v1","id":"following-frames","revision":1,"mode":"steer",
                "clauses":[{"id":"c0","forbid":{"ast":{"language":"python","rule":rules[0],"utils":utils}},
                "violation":{"code":"C0","message":"match","anchor":"witness.span"}}]}).to_string(),
            "json".into(), crate::code_rule_dsl::test_support::CAPABILITIES.into()
        ).unwrap();
        for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 1024] {
            let r = check_work(&p, &rules, &["a=1\na\n"], None, work);
            if work == 1024 {
                assert!(!r["sqlCalls"].as_array().unwrap().is_empty());
            }
        }
    }
}
#[test]
fn parameter_frames_keep_count_text_and_node_memo_inputs_distinct() {
    let source = "1;";
    let tree = ast_grep_core::AstGrep::doc(crate::code_rule_dsl::sql::tests::differential::tape(
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
        source,
    ));
    let node_id = tree
        .root()
        .dfs()
        .find(|node| node.kind() == "number")
        .unwrap()
        .node_id() as i64;
    let count =
        json!({"count":{"has":{"kind":"identifier"},"min":0,"max":0,"as":"P","offset":node_id}});
    let direct = json!({"matches":"leaf"});
    let framed = json!({"all":[{"pattern":"$X"},{"matches":{"frame":{"P":"$X"}}}]});
    let utils =
        json!({"leaf":{"value":{"gt":0}},"frame":{"params":["P"],"rule":{"matches":"leaf"}}});
    for terms in [
        vec![count.clone(), direct.clone(), framed.clone()],
        vec![json!({"not":{"not":framed}}), count, direct],
    ] {
        // Count P carries text, while private parameter P carries a node. Their
        // integer SQL payloads deliberately match; native memo binding kinds differ.
        let rules = [json!({"kind":"number","all":terms})];
        let p = policy_utils(&rules, utils.clone());
        for work in [0, 1, 2, 3, 4, 5, 8, 64] {
            let r = check_work(&p, &rules, &[source], None, work);
            if work == 64 {
                assert_eq!(r["sqlCalls"].as_array().unwrap().len(), 2);
            }
        }
    }
}
