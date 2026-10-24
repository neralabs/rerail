//! Native differential checks for transactional capture constraints.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    matcher::AstMatcher,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};

fn policy(ast: Value) -> CompiledCodeRuleDsl {
    let mut ast = ast;
    ast["language"] = json!("typescript");
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"constraints","revision":1,"mode":"steer",
        "clauses":[{"id":"c0","forbid":{"ast":ast},
        "violation":{"code":"C0","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

#[test]
fn constraint_patterns_keep_nested_captures_and_caller_roots() {
    for ast in [
        json!({"rule":{"pattern":"f($A)"},"constraints":{"A":{"pattern":"g($B)"}}}),
        json!({"rule":{"pattern":"f($A, $B)"},"constraints":{"A":{"pattern":"g($C)"},"B":{"pattern":"g($C)"}}}),
        json!({"rule":{"any":[{"pattern":"f($A)"},{"pattern":"h($B)"}]},"constraints":{"A":{"regex":"^g"}}}),
        json!({"rule":{"pattern":"f($A)"},"constraints":{"A":{"not":{"pattern":"g($B)"}}}}),
    ] {
        let policy = policy(ast);
        let result = super::super::tests::measure(
            &grammar_matchers(&policy)[0],
            &[
                (
                    "a.ts".into(),
                    "f(g(x)); f(g(x),g(x)); f(g(x),g(y)); h(z); f(z);".into(),
                ),
                ("b.ts".into(), "f(g(q)); h(g(r));".into()),
            ],
        );
        assert_eq!(result["matchingStatements"], 1);
        assert_eq!(result["framePublications"], 0);
    }
}

#[test]
fn constraint_unknown_stops_before_later_false_or_failure() {
    for ast in [
        json!({"rule":{"pattern":"f($A, $B)","testUnknown":"u"},
            "constraints":{"A":{"value":{"gt":0}},"B":{"regex":"^never$"}}}),
        json!({"rule":{"pattern":"f($A, $B)"},
            "constraints":{"A":{"testUnknown":"^u$"},"B":{"value":{"gt":0}}}}),
        json!({"rule":{"pattern":"f($A, $B)"},
            "constraints":{"A":{"regex":"^never$"},"B":{"value":{"gt":0}}}}),
    ] {
        let policy = policy(ast.clone());
        for allowance in [0, 1, 2, 8, 64] {
            for fault in [None, Some("2")] {
                check_work(
                    &policy,
                    &[ast["rule"].clone()],
                    &["f(u,2); f(1,2); f(3,4);", "f(u,2);"],
                    fault,
                    allowance,
                );
            }
        }
    }
}

#[test]
fn constraint_budgeted_utilities_commit_only_complete_transactions() {
    let ast = json!({"rule":{"pattern":"f($A, $B)"},
        "utils":{"positive":{"pattern":"$X","value":{"gt":0}}},
        "constraints":{"A":{"all":[{"matches":"positive"},{"matches":"positive"}]},
                       "B":{"not":{"value":{"eq":0}}}}});
    let policy = policy(ast.clone());
    for allowance in [0, 1, 2, 3, 4, 8, 32, 1024] {
        for fault in [None, Some("2")] {
            check_work(
                &policy,
                &[ast["rule"].clone()],
                &["f(1,1); f(2,0); f(x,2); f(-1,2);", "f(3,4);"],
                fault,
                allowance,
            );
        }
    }
}

#[test]
fn constraint_outcomes_preserve_outer_rollback_and_withholding() {
    let constrained = policy(json!({"rule":{"pattern":"f($A, $B)"},
        "constraints":{"A":{"testUnknown":"^u$"},"B":{"regex":"^never$"}}}));
    let kind = policy(json!({"rule":{"kind":"call_expression"}}));
    let same_binding = policy(json!({"rule":{"pattern":"$A","regex":"^never$"}}));
    let independent = policy(json!({"rule":{"regex":"^never$"}}));
    let fallback = policy(json!({"rule":{"pattern":"$F"}}));
    let original = grammar_matchers(&constrained)[0].matcher.clone();
    let kind_matcher = &grammar_matchers(&kind)[0];
    for expression in [
        AstMatcher::Not(Box::new(original.clone())),
        AstMatcher::Any(vec![
            original.clone(),
            grammar_matchers(&fallback)[0].matcher.clone(),
        ]),
        AstMatcher::Not(Box::new(AstMatcher::All(vec![
            original.clone(),
            grammar_matchers(&same_binding)[0].matcher.clone(),
        ]))),
        AstMatcher::Not(Box::new(AstMatcher::All(vec![
            original.clone(),
            grammar_matchers(&independent)[0].matcher.clone(),
        ]))),
    ] {
        let mut grammar = grammar_matchers(&constrained)[0].clone();
        grammar.matcher = AstMatcher::All(vec![kind_matcher.matcher.clone(), expression]);
        grammar.anchors = kind_matcher.anchors.clone();
        grammar.text = None;
        let result = super::super::tests::measure(
            &grammar,
            &[("unknown.ts".into(), "f(u,2); f(v,2);".into())],
        );
        assert_eq!(result["matchingStatements"], 1);
        assert_eq!(result["framePublications"], 0);
    }
}

#[test]
fn constraint_only_count_exports_remain_explicitly_unsupported() {
    let ast = json!({"rule":{"pattern":"f($A)"},
        "constraints":{"A":{"count":{"has":{"kind":"number"},"min":1,"as":"N"}}}});
    let policy = policy(ast);
    let grammar = &grammar_matchers(&policy)[0];
    assert!(
        crate::code_rule_dsl::matcher::CaptureAnalysis::default()
            .counts(&grammar.matcher)
            .is_empty()
    );
    let error = super::compile(grammar)
        .err()
        .expect("internal count publication is not admitted");
    assert!(error.contains("constraint-only count exports"), "{error}");
}

#[test]
fn constraint_core_kinds_bound_root_demand_before_target_effects() {
    for constraint in [json!({"regex":"^1$"}), json!({"value":{"gt":0}})] {
        let ast = json!({"rule":{"pattern":"f($A)"},"constraints":{"A":constraint}});
        let compiled = policy(ast.clone());
        for unrelated in [32, 128, 512] {
            let mut source = String::from("f(1); g(2);\n");
            for index in 0..unrelated {
                source.push_str(&format!("let unused{index} = {index};\n"));
            }
            let measured = super::super::tests::measure(
                &grammar_matchers(&compiled)[0],
                &[("constraint-demand.ts".into(), source.clone())],
            );
            assert_eq!(measured["patternRequests"], 2, "{measured}");
            assert_eq!(measured["matches"], 1, "{measured}");
            assert_eq!(measured["matchingStatements"], 1, "{measured}");
            assert_eq!(measured["framePublications"], 0, "{measured}");
            if constraint.get("value").is_some() {
                for allowance in [0, 1, 2] {
                    for fault in [None, Some("1")] {
                        check_work(
                            &compiled,
                            &[ast["rule"].clone()],
                            &[&source],
                            fault,
                            allowance,
                        );
                    }
                }
            }
        }
    }
}
