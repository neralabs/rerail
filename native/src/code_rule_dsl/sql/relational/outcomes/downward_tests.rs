//! Generic pure downward components, with native candidate filters and source order.
//! Depth samples stop at 128; these tests do not establish native stack-byte parity.
use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn utils(guard: Value) -> Value {
    json!({"u":{"any":[{"kind":"number","regex":"^1$"},
        {"all":[guard,{"has":{"stopBy":"neighbor","matches":"u"}}]}]},
        "wrapper":{"matches":"u"}})
}

fn parentheses() -> Value {
    utils(json!({"kind":"parenthesized_expression"}))
}

fn check(rules: &[Value], utilities: Value, sources: &[&str], work: i64) -> Value {
    let p = policy_utils(rules, utilities);
    let report = check_work(&p, rules, sources, None, work);
    assert_eq!(report["metrics"]["execution"]["matchingStatements"], 1);
    assert_eq!(report["metrics"]["framePublications"], 0);
    report
}

fn successes(report: &Value) -> usize {
    report["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r[4] == 0)
        .count()
}

#[test]
fn downward_preserves_candidate_filters_and_duplicate_call_work() {
    for source in ["(((1)));", "(((2)));", "(((x)));", "(1); (2);"] {
        for term in [
            json!({"matches":"u"}),
            json!({"matches":"wrapper"}),
            json!({"all":[{"matches":"u"},{"matches":"u"}]}),
            json!({"any":[{"matches":"u"},{"matches":"u"}]}),
            json!({"not":{"matches":"u"}}),
        ] {
            let rule = json!({"kind":"parenthesized_expression","all":[term]});
            for work in 0..=20 {
                check(std::slice::from_ref(&rule), parentheses(), &[source], work);
            }
        }
    }
    // The finite number/parenthesis filter skips punctuation in two-valued mode.
    let duplicate =
        json!({"kind":"parenthesized_expression","all":[{"matches":"u"},{"matches":"u"}]});
    let report = check(
        std::slice::from_ref(&duplicate),
        parentheses(),
        &["(1);"],
        4,
    );
    assert_eq!(successes(&report), 1);
    assert_eq!(report["output"][0][5], 0);

    // A regex-only base has no finite kind proof: `(` is a failed recursive
    // invocation before the successful number, so each call costs three entries.
    let unbounded = json!({"u":{"any":[{"regex":"^1$"},
        {"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]},
        "wrapper":{"matches":"u"}});
    for work in 0..=20 {
        check(
            std::slice::from_ref(&duplicate),
            unbounded.clone(),
            &["(1); ((2));"],
            work,
        );
    }
    let report = check(std::slice::from_ref(&duplicate), unbounded, &["(1);"], 6);
    assert_eq!(successes(&report), 1);
    assert_eq!(report["output"][0][5], 0);

    // An unreachable value branch selects three-valued evaluation for the
    // whole caller. Native disables its kind filter even for a kinded base.
    let three = json!({"kind":"parenthesized_expression","any":[
        {"all":[{"matches":"u"},{"matches":"u"}]},
        {"all":[{"regex":"^never$"},{"value":{"eq":99}}]}
    ]});
    let p = policy_utils(std::slice::from_ref(&three), parentheses());
    assert!(crate::code_rule_dsl::test_support::grammar_matchers(&p)[0].three_valued);
    for work in 0..=20 {
        check(
            std::slice::from_ref(&three),
            parentheses(),
            &["(1); ((2));"],
            work,
        );
    }
    let report = check(&[three], parentheses(), &["(1);"], 6);
    assert_eq!(successes(&report), 1);
    assert_eq!(report["output"][0][5], 0);
}

#[test]
fn downward_branching_search_checks_failed_siblings_before_first_success() {
    let utilities = utils(json!({"kind":"array"}));
    let rule = json!({"kind":"array","matches":"u"});
    for source in ["[2,3,[4,1],1];", "[2,[3,4],5];", "[[],[2],[3,[1]]];"] {
        for work in 0..=20 {
            check(
                std::slice::from_ref(&rule),
                utilities.clone(),
                &[source],
                work,
            );
        }
    }
    assert_eq!(
        successes(&check(&[rule], utilities, &["[2,3,[4,1],1];"], 100)),
        2
    );
}

#[test]
fn downward_retains_outer_unknown_capture_and_publication_transactions() {
    let rule = json!({"kind":"parenthesized_expression","any":[
        {"all":[{"pattern":"$X"},{"value":{"eq":99}},{"matches":"u"}]},
        {"all":[{"matches":"wrapper"},{"pattern":"$Y"}]}
    ]});
    for work in 0..=20 {
        check(
            std::slice::from_ref(&rule),
            parentheses(),
            &["(1); (x); (2);"],
            work,
        );
    }
    let rules = [
        json!({"kind":"parenthesized_expression","matches":"u"}),
        json!({"kind":"number","pattern":"$Y"}),
    ];
    for work in 0..=20 {
        check(
            &rules,
            parentheses(),
            &["(1); (((1)));", "((2)); (1);"],
            work,
        );
    }
    let report = check(&rules, parentheses(), &["(1); (((1)));", "((2)); (1);"], 3);
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
fn recursive_components_admit_memoized_bases_and_strict_progress_cycles() {
    for utilities in [
        json!({"u":{"any":[{"value":{"eq":1}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}}),
        json!({"u":{"any":[{"regex":"^1$"},{"has":{"stopBy":"neighbor","matches":"v"}}]},"v":{"has":{"stopBy":"neighbor","matches":"u"}}}),
    ] {
        let p = policy_utils(
            &[json!({"kind":"parenthesized_expression","matches":"u"})],
            utilities,
        );
        let grammars = crate::code_rule_dsl::test_support::grammar_matchers(&p);
        super::compile(&grammars[0]).unwrap();
        for work in [0, 1, 2, 4, 16, 64, 1024] {
            super::publication_tests::check_work(
                &p,
                &[json!({"kind":"parenthesized_expression","matches":"u"})],
                &["(1); ((2)); (((1)));"],
                None,
                work,
            );
        }
    }
}

#[test]
fn downward_capture_base_commits_before_recursive_continuation() {
    let rules = [json!({"kind":"parenthesized_expression","matches":"u"})];
    let utilities = json!({"u":{"any":[{"pattern":"$X"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}});
    for work in 0..=8 {
        check(&rules, utilities.clone(), &["(1); (((2)));"], work);
    }
}

fn sample(report: &Value) -> Value {
    json!({"vmSteps":report["metrics"]["execution"]["vmSteps"],
        "nodeAdvances":report["metrics"]["nodeAdvances"],
        "sqlBytes":report["sql"].as_str().unwrap().len()})
}

fn linear(samples: &[Value]) {
    for pair in samples.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(
            pair[1]["vmSteps"].as_u64().unwrap() <= 6 * pair[0]["vmSteps"].as_u64().unwrap(),
            "{samples:?}"
        );
    }
}

#[test]
fn downward_shared_subtrees_scale_linearly_and_setup_follows_demand() {
    let rule = json!({"kind":"number","inside":{"stopBy":"neighbor","matches":"u"}});
    let mut depths = Vec::new();
    let mut shape = Value::Null;
    for depth in [8usize, 32, 128] {
        let source = format!("{}1{};", "(".repeat(depth), ")".repeat(depth));
        let shared = json!({"any":[{"kind":"number"},{"kind":"parenthesized_expression"}],"all":[{"matches":"u"},{"matches":"u"}]});
        let report = check(&[shared], parentheses(), &[&source], i64::MAX);
        assert_eq!(successes(&report), depth + 1);
        let mut point = sample(&report);
        point["depth"] = json!(depth);
        depths.push(point);
        if shape.is_null() {
            shape = json!({"sql":report["sql"],"explain":report["metrics"]["explain"]});
        }
    }
    linear(&depths);
    let mut unrelated = Vec::new();
    for count in [0, 128, 512] {
        let source = format!("((((1))));\n{}", "let unrelated;\n".repeat(count));
        unrelated.push(sample(&check(
            std::slice::from_ref(&rule),
            parentheses(),
            &[&source],
            i64::MAX,
        )));
    }
    for point in &unrelated {
        assert_eq!(point, &unrelated[0], "{unrelated:?}");
    }
    let mut disconnected = Vec::new();
    for count in [8, 32, 128] {
        let source = format!("((1));\n{}", "((other));\n".repeat(count));
        let report = check(
            std::slice::from_ref(&rule),
            parentheses(),
            &[&source],
            i64::MAX,
        );
        assert_eq!(successes(&report), 1);
        disconnected.push(sample(&report));
    }
    linear(&disconnected);
    let mut widths = Vec::new();
    for width in [8, 32, 128] {
        let source = format!("[{}];", vec!["[2,[3,1]]"; width].join(","));
        let rule = json!({"kind":"array","all":[{"matches":"u"},{"matches":"u"}]});
        let report = check(
            &[rule],
            utils(json!({"kind":"array"})),
            &[&source],
            i64::MAX,
        );
        assert_eq!(successes(&report), 1 + width * 2);
        widths.push(sample(&report));
    }
    linear(&widths);
    let mut early = Vec::new();
    for count in [0, 32, 128] {
        let source = format!("((1));\n{}", "((other));\n".repeat(count));
        let no_roots = "((other));\n".repeat(count + 1);
        for skipped in [false, true] {
            let rule = if skipped {
                json!({"kind":"number","any":[{"regex":".*"},{"matches":"u"}]})
            } else {
                json!({"kind":"number","matches":"u"})
            };
            let report = check(&[rule], parentheses(), &[&source, &no_roots], 0);
            assert_eq!(report["output"][0][4], if skipped { 0 } else { -2 });
            assert_eq!(report["nativeTraces"][1]["candidateVisits"], 0);
            let mut point = sample(&report);
            point["skipped"] = json!(skipped);
            early.push(point);
        }
    }
    for point in &early {
        assert_eq!(
            point,
            &early[usize::from(point["skipped"] == true)],
            "{early:?}"
        );
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_DOWNWARD_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(
                &json!({"sharedSubtrees":depths,"sharedSiblingWidths":widths,
            "unrelatedKinds":unrelated,"disconnectedGuards":disconnected,
            "earlyFailureAndSkip":early,"shape":shape}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
