//! Ordered recursive alternatives and pure child guards retain native reached effects.
use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};
fn recurse() -> Value {
    json!({"kind":"array","has":{"stopBy":"neighbor","not":{"regex":"^[\\[\\],]$"},"matches":"u"}})
}
fn base() -> Value {
    json!({"any":[{"kind":"array","pattern":"$X"},{"kind":"number","pattern":"$X","value":{"eq":1}}]})
}
#[test]
fn recursive_first_and_last_alternatives_choose_different_exact_witnesses() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    for first in [false, true] {
        let arms = if first {
            vec![recurse(), base()]
        } else {
            vec![base(), recurse()]
        };
        let p = policy_utils(&rules, json!({"u":{"any":arms}}));
        for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 128, 1024, i64::MAX] {
            check_work(&p, &rules, &["f([0,[1],2]); f([]); f([0]);"], None, work);
        }
        let r = check_work(&p, &rules, &["f([0,[1],2]);"], Some("2"), i64::MAX);
        assert_eq!(
            r["sqlCalls"],
            if first { json!(["0", "1"]) } else { json!([]) }
        );
    }
}
#[test]
fn recursive_child_prefix_skips_calls_without_losing_guard_order() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let u = json!({"any":[{"kind":"number","value":{"eq":1}},{"kind":"array","has":{"stopBy":"neighbor","not":{"regex":"^(?:[\\[\\],]|0)$"},"matches":"u"}}]});
    let p = policy_utils(&rules, json!({"u":u}));
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 128, 1024, i64::MAX] {
        check_work(&p, &rules, &["f([0,0,[1],2]); f([0,[2],1]);"], None, work);
    }
    let r = check_work(&p, &rules, &["f([0,0,[1],2]);"], Some("0"), i64::MAX);
    assert_eq!(r["sqlCalls"], json!(["1"]));
}
#[test]
fn recursive_first_unknown_capture_branch_does_not_fall_back_to_outer_capture() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let u = json!({"any":[recurse(),{"kind":"number","pattern":"$X","testUnknown":"^0$","value":{"eq":1}},{"kind":"array","pattern":"$X"}]});
    let p = policy_utils(&rules, json!({"u":u}));
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, 1024, i64::MAX] {
        check_work(&p, &rules, &["f([0,1]); f([1]);"], None, work);
    }
}
#[test]
fn recursive_multiple_base_arms_preserve_unknown_binding_provenance() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let u = json!({"any":[{"testUnknown":"^\\["},{"pattern":"$X","regex":"^never$"},recurse()]});
    let p = policy_utils(&rules, json!({"u":u}));
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, 1024, i64::MAX] {
        check_work(&p, &rules, &["f([1]); f([[2]]);"], None, work);
    }
}

#[test]
fn recursive_neighbor_axes_preserve_single_targets_and_document_root_zero() {
    for axis in ["inside", "follows", "precedes"] {
        let rules = [json!({"kind":"number","matches":"u"})];
        let u = json!({"any":[{"kind":"number","value":{"eq":1}},{axis:{"stopBy":"neighbor","matches":"u"}}]});
        let p = policy_utils(&rules, json!({"u":u}));
        for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 128, 1024, i64::MAX] {
            check_work(&p, &rules, &["f([2,3,1]); f([1,2,3]);"], None, work);
        }
    }
    // The final ancestor is document node0. A non-child cursor must distinguish
    // that visited target from its initial sentinel instead of calling it again.
    let rules = [json!({"kind":"identifier","regex":"^x$","matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"number","value":{"eq":1}},{"inside":{"stopBy":"neighbor","matches":"u"}}]}}),
    );
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["f(x);"], None, work);
    }
    // A failing prefix on the one next sibling must not seek past that sibling.
    let rules = [json!({"kind":"number","matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"number","value":{"eq":1}},{"precedes":{"stopBy":"neighbor","not":{"regex":"^,$"},"matches":"u"}}]}}),
    );
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["f([2,1]);"], None, work);
    }
}

#[test]
fn recursive_parent_continuation_transports_captured_inputs_and_outputs() {
    let rules = [json!({"kind":"identifier","regex":"^x$","pattern":"$N","matches":"u"})];
    let u = json!({"any":[{"kind":"array","pattern":"$A","has":{"stopBy":"neighbor","pattern":"$N"}},{"inside":{"stopBy":"neighbor","matches":"u"}}]});
    let p = policy_utils(&rules, json!({"u":u}));
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["f([x]); f((x)); f([[x]]);"], None, work);
    }
}

#[test]
fn recursive_ordered_multiple_arms_and_field_selectors_keep_native_order() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let u = json!({"any":[
        {"kind":"parenthesized_expression","has":{"stopBy":"neighbor","not":{"regex":"^[()\\[\\],]$"},"matches":"u"}},
        {"kind":"identifier","regex":"^a$","pattern":"$X","any":[{"value":{"constant":true}},{"regex":"."}]},
        {"kind":"member_expression","has":{"field":"object","matches":"u"}},
        {"kind":"number","pattern":"$X","value":{"eq":1}},
        {"kind":"array","has":{"stopBy":"neighbor","not":{"regex":"^[()\\[\\],]$"},"matches":"u"}},
        {"kind":"identifier","regex":"^b$"}
    ]});
    let p = policy_utils(&rules, json!({"u":u}));
    let source = "f((a)); f(b.one); f([0,1]); f(bad.one); f([0,0]); f(1); f(0);";
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        check_work(&p, &rules, &[source], None, work);
    }
    for fault in ["a", "1"] {
        check_work(&p, &rules, &[source], Some(fault), i64::MAX);
    }
}

#[test]
fn recursive_parent_field_checks_origin_role_before_entering_parent() {
    let rules = [json!({"kind":"identifier","regex":"^a$","matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"number","value":{"eq":1}},{"inside":{"field":"object","matches":"u"}}]}}),
    );
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["a.b.c; b.a.c; a;"], None, work);
    }
}

#[test]
fn recursive_mixed_direction_alternatives_require_a_separate_progress_contract() {
    let rules = [json!({"matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"number","value":{"eq":1}},{"inside":{"stopBy":"neighbor","matches":"u"}},{"has":{"stopBy":"neighbor","matches":"u"}}]}}),
    );
    let grammars = crate::code_rule_dsl::test_support::grammar_matchers(&p);
    let error = super::compile_job(&[&grammars[0]], 100)
        .err()
        .expect("opposite edges may revisit the same node");
    assert!(
        error.contains("component contract") || error.contains("cyclic"),
        "{error}"
    );
}

#[test]
fn recursive_base_retains_preexisting_capture_written_only_by_another_arm() {
    let rules = [json!({"kind":"number","pattern":"$Y","matches":"u"})];
    let u = json!({"any":[{"kind":"number","pattern":"$X","value":{"eq":1}},{"kind":"string","pattern":"$Y"},{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}]});
    let p = policy_utils(&rules, json!({"u":u}));
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["1;2;1;"], None, work);
    }
}
