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
fn shared_memo_replays_across_double_negation() {
    let r = check(
        json!({"kind":"number","all":[{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}),
        json!({"v":{"value":{"gt":0}}}),
        &["1;", "2;"],
        5,
    );
    assert_eq!(r[3]["output"][0][4], -2);
    assert_eq!(r[4]["output"][0][4], 0);
    assert_eq!(r[4]["output"][0][5], 0);
    assert_eq!(r[4]["sqlCalls"].as_array().unwrap().len(), 2);
}

#[test]
fn shared_memo_preserves_unknown_false_and_branch_reachability() {
    for (kind, source) in [("number", "1;2;"), ("identifier", "x;y;")] {
        for body in [json!({"value":{"gt":0}}), json!({"value":{"eq":99}})] {
            for terms in [
                json!({"all":[{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}),
                json!({"any":[{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}),
                // The first syntactic invocation is unreached. The later one must miss.
                json!({"any":[{"all":[{"regex":"^never$"},{"matches":"v"}]},{"matches":"v"}]}),
                json!({"all":[{"any":[{"regex":".*"},{"matches":"v"}]},{"matches":"v"}]}),
                // A cached answer inside an unreached nested scope must spend no work.
                json!({"all":[{"matches":"v"},{"any":[{"regex":".*"},{"all":[{"matches":"v"},{"matches":"v"}]}]}]}),
                json!({"any":[{"matches":"v"},{"all":[{"regex":"^never$"},{"any":[{"matches":"v"},{"matches":"v"}]}]}]}),
                // Negation rolls back bindings, but not reached answers or charges.
                json!({"any":[{"not":{"matches":"v"}},{"matches":"v"}]}),
            ] {
                let rule = json!({"kind":kind,"all":[terms]});
                check(rule.clone(), json!({"v":body}), &[source, source], 10);
                // Also expose remaining allowance for false outcomes.
                check(
                    json!({"kind":kind,"not":rule}),
                    json!({"v":body}),
                    &[source, source],
                    10,
                );
            }
        }
    }
}

#[test]
fn shared_memo_keys_capture_rollback_and_replays_exports() {
    for (rule, utils, source) in [
        (
            json!({"kind":"number","all":[{"matches":"v"},{"pattern":"$X"},{"not":{"not":{"matches":"v"}}},{"matches":"v"}]}),
            json!({"v":{"value":{"gt":0}}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","all":[{"not":{"not":{"matches":"v"}}},{"matches":"v"},{"matches":"v"}]}),
            json!({"v":{"pattern":"$X","value":{"gt":0}}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","all":[{"not":{"all":[{"pattern":"$X"},{"matches":"v"},{"regex":"^never$"}]}},{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}),
            json!({"v":{"value":{"gt":0}}}),
            "1;2;",
        ),
        (
            json!({"kind":"number","any":[{"all":[{"pattern":"$X"},{"matches":"v"},{"regex":"^never$"}]},{"all":[{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}]}),
            json!({"v":{"value":{"gt":0}}}),
            "1;2;",
        ),
        (
            json!({"kind":"identifier","all":[{"matches":"v"},{"not":{"not":{"matches":"v"}}},{"pattern":"$X"}]}),
            json!({"v":{"pattern":"$X","value":{"gt":0}}}),
            "x;y;",
        ),
        (
            json!({"kind":"arguments","all":[{"not":{"not":{"matches":"v"}}},{"matches":"v"}]}),
            json!({"v":{"count":{"has":{"kind":"number","value":{"gt":0}},"min":1,"as":"N"}}}),
            "f(1,2);",
        ),
        (
            json!({"kind":"number","all":[{"matches":"v"},{"not":{"not":{"matches":"w"}}},{"not":{"not":{"matches":"v"}}},{"matches":"w"}]}),
            json!({"v":{"value":{"gt":0}},"w":{"value":{"gt":0}}}),
            "1;2;",
        ),
    ] {
        check(rule, utils, &[source, source], 20);
    }
}

#[test]
fn shared_memo_keeps_failures_and_decisive_branches_ordered() {
    for rule in [
        json!({"kind":"number","all":[{"matches":"v"},{"not":{"not":{"matches":"v"}}}]}),
        json!({"kind":"number","any":[{"regex":".*"},{"not":{"matches":"v"}},{"matches":"v"}]}),
        json!({"kind":"number","all":[{"regex":"^never$"},{"not":{"matches":"v"}},{"matches":"v"}]}),
        json!({"kind":"number","any":[{"all":[{"regex":"^never$"},{"matches":"v"}]},{"matches":"v"}]}),
    ] {
        let rules = [rule.clone(), rule];
        let p = policy_utils(&rules, json!({"v":{"value":{"gt":0}}}));
        for allowance in [0, 1, 3, 4, 100] {
            check_work(&p, &rules, &["1;2;3;", "4;5;6;"], Some("2"), allowance);
        }
    }
}

#[test]
fn shared_memo_bounds_possible_miss_storage() {
    let mut calls: Vec<_> = (0..10)
        .map(|_| json!({"not":{"not":{"matches":"v"}}}))
        .collect();
    calls[0] = json!({"any":[{"regex":".*"},{"matches":"v"}]});
    let p = policy_utils(
        &[json!({"kind":"number","pattern":"$X","all":calls})],
        json!({"v":{"value":{"gt":0}}}),
    );
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST");
    };
    assert!(
        super::compile_job(&[&matchers[0]], 100)
            .err()
            .unwrap()
            .contains("eight key versions")
    );
}

#[test]
fn shared_memo_profile() {
    let mut samples = vec![];
    for dimension in ["repeats", "roots", "false", "unknown", "skipped"] {
        for size in [2, 8, 32] {
            let repeats = if dimension == "roots" { 2 } else { size };
            let mut calls = vec![json!({"not":{"not":{"matches":"v"}}}); repeats];
            if dimension == "skipped" {
                calls[0] = json!({"all":[{"regex":"^never$"},{"matches":"v"}]});
            }
            let (rule, utils, source) = match dimension {
                "false" | "skipped" => (
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
            let roots = if dimension == "roots" { size } else { 1 };
            assert_eq!(
                r["metrics"]["execution"]["typedReturnCells"],
                if dimension == "skipped" {
                    16 * roots
                } else {
                    12 * roots
                }
            );
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_SHARED_MEMO_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
