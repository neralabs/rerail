//! Native field stop geometry, inclusive ordering, fresh captures, and finite work.
use super::publication_tests::check_work;
use serde_json::{Value, json};
fn check(rule: Value, utils: Value, sources: &[&str]) {
    let policy = super::parse_tests::policy("python", rule.clone(), utils);
    for work in [0, 1, 2, 3, 8, 32, 1000, 10000] {
        for fault in [None, Some("1")] {
            check_work(&policy, std::slice::from_ref(&rule), sources, fault, work);
        }
    }
}
#[test]
fn stopped_field_has_tests_selected_root_then_only_its_immediate_children() {
    for regex in ["^1$", "^2$", "^3$"] {
        check(
            json!({"kind":"assignment","has":{"field":"right","stopBy":{"regex":"^never$"},"kind":"integer","regex":regex,"pattern":"$X"}}),
            json!({}),
            &["a=[1,[2]]\nb=((3))\nc=2\nd=[]\n"],
        );
    }
}
#[test]
fn stopped_field_inside_checks_every_stop_and_tests_inclusive_eligible_parent() {
    let source = "def outer(x: int):\n    a=1\n    class Inner:\n        b=2\n    def inner():\n        c=3\nd=4\n";
    for stop in ["class_definition", "function_definition"] {
        for kind in ["function_definition", "class_definition"] {
            check(
                json!({"kind":"integer","inside":{"field":"body","stopBy":{"kind":stop},"kind":kind,"pattern":"$F"}}),
                json!({}),
                &[source],
            );
        }
    }
    check(
        json!({"kind":"identifier","regex":"^x$","inside":{"field":"body","stopBy":{"kind":"class_definition"},"kind":"function_definition"}}),
        json!({}),
        &[source],
    );
}
#[test]
fn stopped_field_body_and_stop_have_direction_specific_probe_order() {
    for rule in [
        json!({"kind":"assignment","has":{"field":"right","stopBy":{"value":{"gt":0}},"kind":"integer","value":{"gt":0}}}),
        json!({"kind":"assignment","has":{"field":"right","stopBy":{"testUnknown":"^\\["},"kind":"list"}}),
        json!({"kind":"assignment","has":{"field":"right","stopBy":{"testUnknown":"^\\["},"kind":"integer"}}),
        json!({"kind":"integer","inside":{"field":"right","stopBy":{"value":{"gt":0}},"kind":"assignment","pattern":"$P"}}),
        json!({"kind":"integer","inside":{"field":"right","stopBy":{"testUnknown":"."},"kind":"assignment","pattern":"$P"}}),
    ] {
        check(rule, json!({}), &["a=1\nb=[1,2]\nc=3\n"]);
    }
}
#[test]
fn stopped_field_stop_is_fresh_and_never_commits_its_captures() {
    check(
        json!({"kind":"assignment","pattern":"$STOP = $RIGHT","has":{"field":"right","stopBy":{"pattern":"$STOP"},"kind":"integer","pattern":"$FOUND"}}),
        json!({}),
        &["a=[1,2]\nb=1\n"],
    );
    check(
        json!({"kind":"assignment","pattern":"$A = $X","any":[{"has":{"field":"right","stopBy":{"pattern":"$DISCARD"},"kind":"list","pattern":"$FOUND"}},{"pattern":"$FALLBACK"}]}),
        json!({}),
        &["a=[1,2]\nb=1\n"],
    );
}
#[test]
fn stopped_field_utilities_keep_empty_stop_keys_and_reached_work() {
    let utils = json!({"body":{"kind":"integer","value":{"gt":0},"pattern":"$N"},"stop":{"kind":"list","value":{"regex":"never"}}});
    check(
        json!({"kind":"assignment","all":[{"has":{"field":"right","stopBy":{"matches":"stop"},"matches":"body"}},{"has":{"field":"right","stopBy":{"matches":"stop"},"matches":"body"}}]}),
        utils,
        &["a=[1,2]\nb=3\nc=[]\n"],
    );
}
#[test]
fn stopped_field_parsed_frontend_boundary_is_explicit() {
    let document = json!({"format":"rerail.policy/v1","id":"parsed-field","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{"language":"python","rule":{"kind":"string","parse":{"grammar":"annotation","has":{"kind":"integer","inside":{"field":"right","stopBy":{"kind":"string"},"kind":"binary_operator"}}}}}},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]});
    let result = crate::code_rule_dsl::CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".into(),
        crate::code_rule_dsl::test_support::CAPABILITIES.into(),
    );
    assert!(result.is_err());
}
