//! Native sibling-index parity across named geometry, budget fills, and parsed documents.
use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn check(rule: Value, utils: Value, source: &str, allowance: i64) -> Value {
    let p = policy_utils(std::slice::from_ref(&rule), utils);
    check_work(&p, std::slice::from_ref(&rule), &[source], None, allowance)
}
#[test]
fn named_sibling_an_b_positions_cover_reverse_roots_and_anonymous_nodes() {
    for position in [
        json!(0),
        json!(1),
        json!(2),
        json!(2147483647),
        json!("2n+1"),
        json!("2n"),
        json!("2n+1"),
        json!("-n+3"),
        json!("n-2"),
        json!("-2n+5"),
    ] {
        for reverse in [false, true] {
            let rule = json!({"nthChild":{"position":position,"reverse":reverse}});
            check(rule, json!({}), "f(a, /* comment */ b, [c,d]); x;", 0);
        }
    }
}
#[test]
fn pure_of_rule_classification_spends_once_per_named_parent_and_caches_empty_lists() {
    for of_rule in [
        json!({"kind":"identifier"}),
        json!({"regex":"^[abc]$"}),
        json!({"pattern":"a"}),
        json!({"not":{"kind":"comment"}}),
        json!({"kind":"string"}),
        json!({"all":[{"kind":"identifier"},{"nthChild":"2n+1"}]}),
    ] {
        for reverse in [false, true] {
            let nth = json!({"nthChild":{"position":1,"reverse":reverse,"ofRule":of_rule}});
            for rule in [nth.clone(), json!({"all":[nth.clone(),nth.clone()]})] {
                for allowance in [0, 1, 2, 3, 4, 6, 10, 20, 1000] {
                    check(
                        rule.clone(),
                        json!({}),
                        "f(a, /* comment */ b, c); g(x);",
                        allowance,
                    );
                }
            }
        }
    }
}
#[test]
fn sibling_cache_fills_are_excluded_from_utility_replay_cost() {
    let utils = json!({"v":{"all":[{"nthChild":{"position":1,"ofRule":{"kind":"identifier"}}},{"value":{"follow":true,"eq":1}}]}});
    let rule = json!({"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}]});
    for allowance in [0, 1, 2, 3, 4, 8, 16, 64, 1000] {
        check(
            rule.clone(),
            utils.clone(),
            "const a=1; const b=2; f(a,b,a);",
            allowance,
        );
    }
}
#[test]
fn parsed_sibling_classifications_repeat_cost_and_do_not_become_cache_fills() {
    let rule = json!({"kind":"string","all":[{"matches":"p"},{"matches":"p"}]});
    let utils = json!({"nth":{"nthChild":{"position":1,"reverse":true,"ofRule":{"kind":"identifier"}}},"p":{"parse":{"grammar":"annotation","has":{"kind":"identifier","matches":"nth","stopBy":"end"}}}});
    let p = super::parse_tests::policy("python", rule.clone(), utils);
    for allowance in [0, 1, 2, 3, 8, 16, 64, 1000, 10000] {
        check_work(
            &p,
            std::slice::from_ref(&rule),
            &["a: 'tuple[int,str]'\nb: 'list[int]'\nc='not an annotation'\n"],
            None,
            allowance,
        );
    }
}
#[test]
fn sibling_of_rule_refuses_value_or_capture_callbacks_before_execution() {
    for of_rule in [
        json!({"value":{"eq":1}}),
        json!({"pattern":"$X"}),
        json!({"testUnknown":".*"}),
    ] {
        let rule = json!({"nthChild":{"position":1,"ofRule":of_rule}});
        let p = policy_utils(std::slice::from_ref(&rule), json!({}));
        let grammar = &crate::code_rule_dsl::test_support::grammar_matchers(&p)[0];
        assert!(
            super::compile(grammar)
                .err()
                .unwrap()
                .contains("ofRule requires")
        );
    }
}

#[test]
fn sibling_classification_identity_is_shared_across_direct_framed_and_scheduled_calls() {
    let utils = json!({
        "nth":{"nthChild":{"position":1,"ofRule":{"kind":"identifier"}}},
        "frame":{"params":["P"],"rule":{"all":[{"pattern":"$P"},{"matches":"nth"}]}},
        "walk":{"any":[{"kind":"identifier","all":[{"matches":"nth"},{"regex":"^yes$"}]},{"has":{"matches":"walk","stopBy":"end"}},{"has":{"matches":"walk","stopBy":"neighbor"}}]}
    });
    let rule = json!({"kind":"identifier","any":[{"all":[{"pattern":"$X"},{"matches":"nth"},{"matches":{"frame":{"P":"$X"}}},{"matches":"walk"}]},{"all":[{"regex":"^never$"},{"value":{"eq":0}}]}]});
    for allowance in [0, 1, 2, 3, 4, 5, 8, 12, 20, 64, 1000] {
        check(
            rule.clone(),
            utils.clone(),
            "[yes,no]; [no,yes]; f(yes,no);",
            allowance,
        );
    }
}
