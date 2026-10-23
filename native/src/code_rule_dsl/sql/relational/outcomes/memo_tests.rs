use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};
fn check(rule: Value, utils: Value, sources: &[&str], max: i64) -> Vec<Value> {
    let rules = [rule, json!({"kind":"number","pattern":"$Y"})];
    let p = policy_utils(&rules, utils);
    (0..=max)
        .map(|work| check_work(&p, &rules, sources, None, work))
        .collect()
}
#[test]
fn memo_replays_cost_once_per_reached_invocation() {
    let result = check(
        json!({"kind":"number","all":[{"matches":"v"},{"matches":"v"}]}),
        json!({"v":{"value":{"gt":0}}}),
        &["1;", "2;"],
        5,
    );
    for allowance in [2, 3] {
        assert_eq!(result[allowance]["output"][0][4], -2);
    }
    assert_eq!(result[4]["output"][0][5], 0);
    for r in &result {
        assert_eq!(
            r["sqlCalls"].as_array().unwrap().len(),
            if r["allowance"] == 0 { 0 } else { 2 }
        );
    }
}
#[test]
fn memo_replays_false_unknown_and_count_costs() {
    for (rule, utils, source) in [
        (
            json!({"kind":"number","any":[{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"value":{"eq":99}}}),
            "1;2;3;",
        ),
        (
            json!({"kind":"identifier","any":[{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"value":{"gt":0}}}),
            "x;y;z;",
        ),
        (
            json!({"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"value":{"gt":0}}}),
            "x;y;z;",
        ),
        (
            json!({"kind":"arguments","all":[{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"count":{"has":{"kind":"number","value":{"gt":0}},"min":1,"as":"N"}}}),
            "f(1,2);",
        ),
    ] {
        check(rule, utils, &[source, source], 18);
    }
}
#[test]
fn memo_keys_include_all_inputs_and_replay_exports() {
    for (rule, utils) in [
        (
            json!({"kind":"number","all":[{"matches":"v"},{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"pattern":"$X","value":{"gt":0}}}),
        ),
        (
            json!({"kind":"number","all":[{"matches":"v"},{"pattern":"$X"},{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"value":{"gt":0}}}),
        ),
        (
            json!({"kind":"number","pattern":"$X","all":[{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"pattern":"$X","value":{"gt":0}}}),
        ),
        (
            json!({"kind":"number","all":[{"value":{"gt":0}},{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"value":{"gt":0}}}),
        ),
    ] {
        check(rule, utils, &["1;2;3;", "4;5;6;"], 15);
    }
}

#[test]
fn memo_distinguishes_utilities_and_retires_answers_independently() {
    check(
        json!({"kind":"number","all":[{"matches":"v"},{"matches":"w"},{"matches":"v"},{"matches":"w"}]}),
        json!({"v":{"value":{"gt":0}},"w":{"value":{"gt":0}}}),
        &["1;2;3;", "4;5;6;"],
        19,
    );
}

#[test]
fn memo_keeps_failures_and_unreached_calls_out_of_the_cache() {
    for rule in [
        json!({"kind":"number","all":[{"matches":"v"},{"matches":"v"}]}),
        json!({"kind":"number","any":[{"regex":".*"},{"matches":"v"},{"matches":"v"}]}),
        json!({"kind":"number","all":[{"regex":"^never$"},{"matches":"v"},{"matches":"v"}]}),
        json!({"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}]}),
    ] {
        let rules = [rule.clone(), rule];
        let p = policy_utils(&rules, json!({"v":{"pattern":"$X","value":{"gt":0}}}));
        for allowance in [0, 1, 3, 4, 100] {
            check_work(&p, &rules, &["1;2;3;x;", "4;5;6;y;"], Some("2"), allowance);
        }
    }
}

#[test]
fn memo_rejects_unproved_scope_sharing() {
    for (rule, utils) in [
        (
            json!({"kind":"arguments","has":{"matches":"v"}}),
            json!({"v":{"value":{"gt":0}}}),
        ),
        (
            json!({"kind":"number","all":[{"matches":"outer"},{"matches":"outer"}]}),
            json!({"outer":{"matches":"v"},"v":{"value":{"gt":0}}}),
        ),
    ] {
        let p = policy_utils(&[rule], utils);
        let crate::code_rule_dsl::model::ClauseKind::Forbid(
            crate::code_rule_dsl::model::Query::Ast { matchers, .. },
        ) = &p.inner.clauses[0].kind
        else {
            panic!("AST")
        };
        assert!(
            super::compile_job(&[&matchers[0]], 100)
                .err()
                .unwrap()
                .contains("utility memo:")
        );
    }
}

#[test]
fn memo_bounds_typed_answer_storage_before_execution() {
    let utils: serde_json::Map<String, Value> = (0..9)
        .map(|i| (format!("u{i}"), json!({"value":{"eq":99}})))
        .collect();
    let calls: Vec<_> = (0..18)
        .map(|i| json!({"matches":format!("u{}",i%9)}))
        .collect();
    let p = policy_utils(&[json!({"kind":"number","any":calls})], json!(utils));
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

#[test]
fn memo_profile() {
    let mut samples = vec![];
    for dimension in ["repeats", "roots", "false", "unknown"] {
        for size in [2, 8, 32] {
            let repeats = if dimension == "roots" { 2 } else { size };
            let calls = vec![json!({"matches":"v"}); repeats];
            let (rule, utils, source) = match dimension {
                "false" => (
                    json!({"kind":"number","any":calls}),
                    json!({"v":{"value":{"eq":99}}}),
                    "1;".into(),
                ),
                "unknown" => (
                    json!({"kind":"identifier","any":calls}),
                    json!({"v":{"value":{"gt":0}}}),
                    "x;".into(),
                ),
                _ => (
                    json!({"kind":"number","all":calls}),
                    json!({"v":{"value":{"gt":0}}}),
                    "1;".repeat(if dimension == "roots" { size } else { 1 }),
                ),
            };
            let rules = [rule];
            let p = policy_utils(&rules, utils);
            let mut r = check_work(&p, &rules, &[&source, &source], None, 1000);
            assert_eq!(
                r["sqlCalls"].as_array().unwrap().len(),
                2 * if dimension == "roots" { size } else { 1 }
            );
            // Repeated identical calls must not copy child return rows again.
            assert_eq!(
                r["metrics"]["execution"]["typedReturnCells"],
                12 * if dimension == "roots" { size } else { 1 }
            );
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_MEMO_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
