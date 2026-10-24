//! Integration contracts for shared, capture-free functional utility chains.
use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn parent_utils() -> Value {
    json!({
        "u":{"any":[
            {"inside":{"stopBy":"neighbor","kind":"call_expression"}},
            {"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"u"}}
        ]},
        "wrapper":{"matches":"u"}
    })
}

fn check(rules: &[Value], utils: Value, sources: &[&str], allowance: i64) -> Value {
    let p = policy_utils(rules, utils);
    let report = check_work(&p, rules, sources, None, allowance);
    assert_eq!(report["metrics"]["execution"]["matchingStatements"], 1);
    assert_eq!(report["metrics"]["framePublications"], 0);
    report
}

fn successes(report: &Value) -> usize {
    report["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r[4].as_i64() == Some(0))
        .count()
}

#[test]
fn parent_chain_preserves_finite_work_and_duplicate_invocation_costs() {
    for source in ["((1))();", "((1));"] {
        for terms in [
            json!({"matches":"u"}),
            json!({"matches":"wrapper"}),
            json!({"all":[{"matches":"u"},{"matches":"u"}]}),
            json!({"any":[{"matches":"u"},{"matches":"u"}]}),
            json!({"not":{"matches":"u"}}),
        ] {
            let rule = json!({"kind":"number","all":[terms]});
            for allowance in 0..=12 {
                check(
                    std::slice::from_ref(&rule),
                    parent_utils(),
                    &[source],
                    allowance,
                );
            }
        }
    }
    let rule = json!({"kind":"number","all":[{"matches":"u"},{"matches":"u"}]});
    let report = check(&[rule], parent_utils(), &["((1))();"], 6);
    assert_eq!(successes(&report), 1);
    assert_eq!(
        report["output"][0][5], 0,
        "each invocation charges all three entries"
    );
    assert_eq!(report["relations"]["recursiveComponents"], 1);
}

#[test]
fn chain_calls_preserve_short_circuiting_and_outer_unknown_capture_rollback() {
    let rules = [
        json!({"kind":"number","any":[{"regex":".*"},{"matches":"u"}]}),
        json!({"kind":"number","all":[{"regex":"^never$"},{"matches":"u"}]}),
    ];
    let report = check(&rules, parent_utils(), &["((1))();"], 0);
    assert_eq!(successes(&report), 1);
    for allowance in 0..=12 {
        let rule = json!({"all":[{"any":[{"kind":"number"},{"kind":"identifier"}]},{"any":[
            {"all":[{"pattern":"$X"},{"matches":"u"},{"value":{"eq":99}}]},
            {"all":[{"matches":"u"},{"pattern":"$Y"}]}
        ]}]});
        // An identifier's value is unknown; both callers retain ordinary outcome semantics.
        check(&[rule], parent_utils(), &["((1))(); ((x))();"], allowance);
    }
}

#[test]
fn shared_chain_keeps_clause_file_publication_order_and_failure_scope() {
    let rules = [
        json!({"kind":"number","matches":"u"}),
        json!({"kind":"number","pattern":"$Y"}),
    ];
    for allowance in 0..=12 {
        check(
            &rules,
            parent_utils(),
            &["1(); ((2))();", "((3)); 4();"],
            allowance,
        );
    }
    let report = check(&rules, parent_utils(), &["1(); ((2))();", "((3)); 4();"], 2);
    assert!(
        report["nativeTraces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["clause"] == 0 && t["provisionalMatches"].as_u64().unwrap() > 0)
    );
    assert!(
        report["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[0] == 1 && r[4] == 0)
    );
}

#[test]
fn sibling_chains_use_named_statements_and_stop_at_anonymous_commas() {
    for (axis, seed) in [("follows", "^a;$"), ("precedes", "^c;$")] {
        let mut edge = json!({});
        edge[axis] = json!({"stopBy":"neighbor","kind":"expression_statement","matches":"u"});
        let utils = json!({"u":{"any":[{"regex":seed},edge]}});
        let rule = json!({"kind":"expression_statement","matches":"u"});
        for allowance in 0..=12 {
            check(
                std::slice::from_ref(&rule),
                utils.clone(),
                &["a;b;c;", "d;e;"],
                allowance,
            );
        }
        assert_eq!(
            successes(&check(&[rule], utils, &["a;b;c;", "d;e;"], 100)),
            3
        );
    }
    // Public kind selectors admit named kinds. A comma cannot be smuggled into
    // the guard: immediate sibling traversal must stop at the anonymous token.
    for (axis, seed) in [("follows", "^1$"), ("precedes", "^3$")] {
        let mut edge = json!({});
        edge[axis] = json!({"stopBy":"neighbor","kind":"number","matches":"u"});
        let utils = json!({"u":{"any":[{"regex":seed},edge]}});
        let rule = json!({"kind":"number","matches":"u"});
        assert_eq!(
            successes(&check(&[rule], utils, &["[1,2,3]; [4,5];"], 100)),
            1
        );
    }
}

#[test]
fn chains_keep_distinct_components_empty_guards_and_memo_refusal_explicit() {
    let mut utils = parent_utils();
    utils["v"] = json!({"any":[{"regex":"^1$"},{"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"v"}}]});
    let rules = [
        json!({"kind":"number","matches":"u"}),
        json!({"kind":"number","matches":"v"}),
        json!({"kind":"number","all":[{"matches":"u"},{"matches":"v"}]}),
    ];
    let report = check(&rules, utils, &["((1))();", "((2));"], 100);
    assert_eq!(report["relations"]["recursiveComponents"], 4);
    let utils = json!({"u":{"any":[{"regex":"^never$"},{"inside":{"stopBy":"neighbor","all":[{"kind":"number"},{"kind":"parenthesized_expression"},{"matches":"u"}]}}]}});
    let rule = json!({"kind":"number","not":{"matches":"u"}});
    let report = check(&[rule], utils, &["((1));"], 1);
    assert_eq!(successes(&report), 1);
    assert_eq!(report["output"][0][5], 0);

    let mut utils = parent_utils();
    utils["memo"] = json!({"all":[{"value":{"constant":true}},{"matches":"u"}]});
    let rule = json!({"kind":"number","all":[{"matches":"memo"},{"matches":"memo"}]});
    for work in [0, 1, 2, 4, 8, 32, 128, 1024] {
        check(
            std::slice::from_ref(&rule),
            utils.clone(),
            &["((1))(); ((2));"],
            work,
        );
    }
}

#[test]
fn chain_suffix_setup_is_shared_linear_and_independent_of_unrelated_kinds() {
    let rule = json!({"kind":"number","all":[{"matches":"u"},{"matches":"u"}]});
    let mut samples = Vec::new();
    for depth in [8usize, 32, 128] {
        // All parentheses are candidate roots, so their paths share suffixes.
        let shared = json!({"any":[{"kind":"number"},{"kind":"parenthesized_expression"}],"all":[{"matches":"u"},{"matches":"u"}]});
        let source = format!("{}1{}();", "(".repeat(depth), ")".repeat(depth));
        let report = check(&[shared], parent_utils(), &[&source], i64::MAX);
        assert_eq!(successes(&report), depth + 1);
        assert_eq!(report["relations"]["recursiveComponents"], 1);
        samples.push(json!({"depth":depth,"sqlBytes":report["sql"].as_str().unwrap().len(),"vmSteps":report["metrics"]["execution"]["vmSteps"]}));
    }
    for pair in samples.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(
            pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap(),
            "{samples:?}"
        );
    }
    let mut unrelated = Vec::new();
    for count in [0, 128, 512] {
        // No numbers or parentheses in these declarations: neither roots nor guard keys grow.
        let source = format!("((((1))))();\n{}", "let unrelated;\n".repeat(count));
        let report = check(
            std::slice::from_ref(&rule),
            parent_utils(),
            &[&source],
            i64::MAX,
        );
        unrelated.push(json!({"declarations":count,"vmSteps":report["metrics"]["execution"]["vmSteps"],"nodeAdvances":report["metrics"]["nodeAdvances"],"sqlBytes":report["sql"].as_str().unwrap().len()}));
    }
    for sample in &unrelated {
        assert_eq!(sample["vmSteps"], unrelated[0]["vmSteps"], "{unrelated:?}");
        assert_eq!(
            sample["nodeAdvances"], unrelated[0]["nodeAdvances"],
            "{unrelated:?}"
        );
        assert_eq!(sample["sqlBytes"], unrelated[0]["sqlBytes"]);
    }
    let mut disconnected = Vec::new();
    let mut shape = Value::Null;
    for count in [8, 32, 128] {
        let source = format!("((1))();\n{}", "((other));\n".repeat(count));
        let report = check(
            std::slice::from_ref(&rule),
            parent_utils(),
            &[&source],
            i64::MAX,
        );
        assert_eq!(successes(&report), 1);
        let explain = report["metrics"]["explain"].as_array().unwrap();
        assert!(
            explain.iter().any(|row| {
                let row = row.as_str().unwrap();
                row.contains("SEARCH l") && row.contains("file=?") && row.contains("next=?")
            }),
            "reverse suffix lookup must index (file,next): {explain:?}"
        );
        assert!(
            explain.iter().any(|row| {
                let row = row.as_str().unwrap();
                row.contains("SEARCH s") && row.contains("file=?") && row.contains("node=?")
            }),
            "invocation lookup must index (file,node): {explain:?}"
        );
        disconnected.push(json!({"guardGroups":count,"vmSteps":report["metrics"]["execution"]["vmSteps"],"nodeAdvances":report["metrics"]["nodeAdvances"],"sqlBytes":report["sql"].as_str().unwrap().len()}));
        if shape.is_null() {
            shape = json!({"sql":report["sql"],"explain":report["metrics"]["explain"]});
        }
    }
    for pair in disconnected.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(
            pair[1]["vmSteps"].as_u64().unwrap() <= 6 * pair[0]["vmSteps"].as_u64().unwrap(),
            "{disconnected:?}"
        );
    }
    let skipped = json!({"kind":"number","any":[{"regex":".*"},{"matches":"u"}]});
    let zero = check(&[skipped], parent_utils(), &["((1))();", "((other));"], 0);
    assert_eq!(successes(&zero), 1);
    assert_eq!(zero["nativeTraces"][0]["remaining"], 0);
    assert_eq!(zero["nativeTraces"][1]["candidateVisits"], 0);
    let demand = json!({"vmSteps":zero["metrics"]["execution"]["vmSteps"],"nodeAdvances":zero["metrics"]["nodeAdvances"],"nativeTraces":zero["nativeTraces"],"recursiveComponents":zero["relations"]["recursiveComponents"]});
    let mut early = Vec::new();
    for count in [0, 32, 128] {
        let source = format!("((1))();\n{}", "((other));\n".repeat(count));
        let no_roots = "((other));\n".repeat(count + 1);
        for skipped in [false, true] {
            let rule = if skipped {
                json!({"kind":"number","any":[{"regex":".*"},{"matches":"u"}]})
            } else {
                json!({"kind":"number","matches":"u"})
            };
            let report = check(&[rule], parent_utils(), &[&source, &no_roots], 0);
            assert_eq!(report["output"][0][4], if skipped { 0 } else { -2 });
            assert_eq!(report["nativeTraces"][1]["candidateVisits"], 0);
            early.push(json!({"guardGroups":count,"skipped":skipped,"vmSteps":report["metrics"]["execution"]["vmSteps"],"nodeAdvances":report["metrics"]["nodeAdvances"]}));
        }
    }
    // Neither a skipped nor an exhausted invocation may prepare its guard domain.
    for sample in &early {
        let baseline = &early[usize::from(sample["skipped"] == true)];
        assert_eq!(sample["vmSteps"], baseline["vmSteps"], "{early:?}");
        assert_eq!(
            sample["nodeAdvances"], baseline["nodeAdvances"],
            "{early:?}"
        );
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_CHAIN_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(
                &json!({"sharedSuffixes":samples,"unrelatedKinds":unrelated,"disconnectedGuards":disconnected,"demand":demand,"shape":shape,"earlyFailureAndSkip":early}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
