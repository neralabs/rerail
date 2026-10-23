//! Actual native allowances, not injected faults or VM instruction limits.
use super::publication_tests::{check_work, policy};
use serde_json::{Value, json};

fn check(rules: &[Value], sources: &[&str], maximum: i64) -> Vec<Value> {
    let p = policy(rules);
    (0..=maximum)
        .map(|work| {
            let mut result = check_work(&p, rules, sources, None, work);
            result["allowance"] = json!(work);
            result
        })
        .collect()
}
fn total() -> Value {
    json!({"kind":"number","pattern":"$Y"})
}
#[test]
fn work_budget_crosses_roots_and_isolates_clause_file_pairs() {
    let rules = [
        json!({"kind":"number","pattern":"$X","value":{"gt":0}}),
        total(),
    ];
    let results = check(&rules, &["1;2;3;", "4;5;6;"], 4);
    for (allowance, result) in results.iter().enumerate() {
        assert_eq!(
            result["sqlCalls"].as_array().unwrap().len(),
            2 * (allowance + 1).min(3)
        );
        assert_eq!(
            result["output"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r[4] == -4)
                .count(),
            if allowance < 3 { 2 } else { 0 }
        );
    }
    for rules in [
        [total(), rules[0].clone()],
        [rules[0].clone(), rules[0].clone()],
    ] {
        check(&rules, &["4;5;6;", "1;2;3;"], 4);
    }
}
#[test]
fn work_budget_preserves_reached_boolean_and_unknown_work() {
    for rule in [
        json!({"kind":"number","any":[{"regex":".*"},{"value":{"gt":0}}]}),
        json!({"kind":"number","not":{"value":{"gt":0}}}),
        json!({"kind":"number","any":[{"value":{"eq":99}},{"value":{"gt":0}}]}),
        json!({"kind":"identifier","all":[{"value":{"eq":1}},{"pattern":"$X"}]}),
        json!({"kind":"identifier","any":[{"value":{"eq":1}},{"pattern":"$X"}]}),
    ] {
        check(&[rule, total()], &["1;2;3; a;b;c;", "4;5;6; x;y;z;"], 8);
    }
}
#[test]
fn work_budget_charges_count_visits_before_candidate_tests() {
    for count in [
        json!({"has":{"kind":"number","value":{"gt":0}},"min":1}),
        json!({"has":{"kind":"number"},"min":0,"max":5}),
        json!({"has":{"kind":"number","value":{"gt":0}},"min":0,"max":5,"as":"N"}),
        json!({"has":{"kind":"identifier","testUnknown":"^u"},"min":1}),
    ] {
        check(
            &[json!({"kind":"arguments","count":count}), total()],
            &["f(1,2,3); g(u,a);", "h(4,5,6); k(u,b);"],
            14,
        );
    }
}
#[test]
fn work_budget_flows_through_nested_traversal_and_stop() {
    for rule in [
        json!({"kind":"arguments","has":{"kind":"array","pattern":"$A","has":{"kind":"number","pattern":"$X","value":{"eq":2}},"stopBy":"neighbor"}}),
        json!({"kind":"arguments","count":{"has":{"kind":"array","has":{"kind":"number","value":{"gt":0}}},"min":0,"max":5,"as":"N"}}),
        json!({"kind":"number","precedes":{"kind":"number","value":{"eq":3},"stopBy":{"value":{"eq":2}}}}),
    ] {
        check(&[rule, total()], &["f([1,2,3]);", "g([4,5,6]);"], 14);
    }
}
#[test]
fn work_budget_rejects_negative_allowance() {
    let p = policy(&[total()]);
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    assert!(
        super::compile_job(&[&matchers[0]], -1)
            .err()
            .unwrap()
            .contains("negative work")
    );
}
#[test]
fn work_budget_keeps_inlined_utility_entry_charges() {
    for rule in [
        json!({"kind":"number","matches":"number"}),
        json!({"kind":"number","all":[{"matches":"number"},{"matches":"number"}]}),
        json!({"kind":"arguments","has":{"matches":"number"}}),
        json!({"kind":"arguments","count":{"has":{"matches":"number"},"min":1}}),
    ] {
        let rules = [json!({"all":[{"testUnknown":"^never$"},rule]}), total()];
        let p = super::publication_tests::policy_utils(&rules, json!({"number":{"kind":"number"}}));
        for work in 0..=12 {
            check_work(&p, &rules, &["f(1,2,3);", "g(4,5,6);"], None, work);
        }
    }
}
#[test]
fn work_budget_rejects_value_bearing_utility_memo_replay() {
    let rules = [json!({"kind":"number","all":[{"matches":"v"},{"matches":"v"}]})];
    let p = super::publication_tests::policy_utils(&rules, json!({"v":{"value":{"gt":0}}}));
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    assert!(
        super::compile_job(&[&matchers[0]], 3)
            .err()
            .unwrap()
            .contains("memo charge replay")
    );
    // Native's second utility invocation replays two steps, even though the value is cached.
    let doc = crate::code_rule_dsl::sql::tests::differential::tape(matchers[0].language, "1;");
    let tree = ast_grep_core::AstGrep::doc(doc.clone());
    let _scopes = crate::code_rule_dsl::matcher::FileScopeSlot::install(
        Default::default(),
        Default::default(),
        false,
    );
    let mut work = 3;
    let result = crate::code_rule_dsl::matcher::with_file_memo(&tree, || {
        crate::code_rule_dsl::matcher::run_clause(
            &tree,
            &matchers[0],
            None,
            doc.transducer_len(),
            &mut work,
            |_, _, _| Ok::<_, ()>(true),
        )
    })
    .unwrap();
    assert!(matches!(
        result,
        Err(crate::code_rule_dsl::matcher::Stopped::Limit(
            crate::code_rule_dsl::matcher::UtilLimit::Work
        ))
    ));
    assert_eq!(work, 0);
}
#[test]
fn work_budget_rejects_unproved_two_valued_candidate_pruning() {
    let rules = [json!({"kind":"arguments","has":{"matches":"n"}})];
    let p = super::publication_tests::policy_utils(&rules, json!({"n":{"kind":"number"}}));
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    assert!(
        super::compile_job(&[&matchers[0]], 1)
            .err()
            .unwrap()
            .contains("native candidate pruning")
    );
}
#[test]
fn work_budget_profile() {
    let mut samples = vec![];
    for dimension in ["roots", "early-limit", "count", "clauses"] {
        for size in [4, 16, 64] {
            let value = json!({"kind":"number","pattern":"$X","value":{"gt":0}});
            let (rules, source, allowance) = match dimension {
                "clauses" => (vec![value; size / 2], "1;".into(), 1),
                "count" => (
                    vec![
                        json!({"kind":"arguments","count":{"has":{"kind":"number","value":{"gt":0}},"min":0,"max":size+1,"as":"N"}}),
                        total(),
                    ],
                    format!("f({});", vec!["1"; size].join(",")),
                    (3 * size + 1) as i64,
                ),
                _ => (
                    vec![value, total()],
                    "1;".repeat(size),
                    if dimension == "early-limit" {
                        0
                    } else {
                        size as i64
                    },
                ),
            };
            let p = policy(&rules);
            let mut r = check_work(&p, &rules, &[&source, &source], None, allowance);
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
                    <= 5 * pair[0]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
            );
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_WORK_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
